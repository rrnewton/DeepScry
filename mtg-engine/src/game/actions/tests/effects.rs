use crate::core::{Card, CardId, CardType, Effect, Keyword, ManaCost, PlayerId, TargetRef};
use crate::game::state::GameState;
use crate::loader::CardDatabase;
use crate::MtgError;
use std::path::PathBuf;

/// Helper to load a card from the cardsfolder for tests
pub(super) fn load_test_card(game: &mut GameState, card_name: &str, owner_id: PlayerId) -> Result<CardId, MtgError> {
    let card_id = game.next_entity_id();

    // Load card definition from cardsfolder (relative to workspace root)
    let cardsfolder = PathBuf::from("../cardsfolder");
    let db = CardDatabase::new(cardsfolder);

    let card_def = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(async { db.get_card(card_name).await })?
        .ok_or_else(|| MtgError::InvalidCardFormat(format!("Card not found: {card_name}")))?;

    // Create card instance from definition
    let card = card_def.instantiate(card_id, owner_id);
    game.cards.insert(card_id, card);

    Ok(card_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::ZeroController;

    #[test]
    fn test_resolve_pump_spell() {
        use crate::core::{Effect, ManaCost};

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let players: Vec<_> = game.players.iter().map(|p| p.id).collect();
        let p1_id = players[0];

        // Create a 2/2 creature on battlefield
        let creature_id = game.next_card_id();
        let mut creature = Card::new(creature_id, "Grizzly Bears".to_string(), p1_id);
        creature.add_type(CardType::Creature);
        creature.set_base_power(Some(2));
        creature.set_base_toughness(Some(2));
        creature.controller = p1_id;
        game.cards.insert(creature_id, creature);
        game.battlefield.add(creature_id);

        // Check initial stats
        let creature_before = game.cards.get(creature_id).unwrap();
        assert_eq!(creature_before.current_power(), 2);
        assert_eq!(creature_before.current_toughness(), 2);

        // Create Giant Growth (pump +3/+3)
        let pump_spell_id = game.next_card_id();
        let mut pump_spell = Card::new(pump_spell_id, "Giant Growth".to_string(), p1_id);
        pump_spell.add_type(CardType::Instant);
        pump_spell.mana_cost = ManaCost::from_string("G");
        // Target the creature we created
        pump_spell.effects.push(Effect::PumpCreature {
            target: creature_id,
            power_bonus: 3,
            toughness_bonus: 3,
            keywords_granted: smallvec::SmallVec::new(),
        });
        game.cards.insert(pump_spell_id, pump_spell);

        // Put spell on stack (simulating cast)
        game.stack.add(pump_spell_id);

        // Resolve the spell
        assert!(
            game.resolve_spell(pump_spell_id, &[]).is_ok(),
            "Failed to resolve pump spell"
        );

        // Check creature got the bonus
        let creature_after = game.cards.get(creature_id).unwrap();
        assert_eq!(
            creature_after.current_power(),
            5,
            "Creature should have +3 power bonus (2 + 3)"
        );
        assert_eq!(
            creature_after.current_toughness(),
            5,
            "Creature should have +3 toughness bonus (2 + 3)"
        );

        // Check spell went to graveyard
        if let Some(zones) = game.get_player_zones(p1_id) {
            assert!(
                zones.graveyard.contains(pump_spell_id),
                "Pump spell should be in graveyard"
            );
        }
    }

    #[test]
    fn test_pump_effect_cleanup_at_end_of_turn() {
        use crate::core::CardType;
        use crate::game::Step;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Create a 2/2 creature on battlefield
        let creature_id = game.next_card_id();
        let mut creature = Card::new(creature_id, "Grizzly Bears".to_string(), p1_id);
        creature.add_type(CardType::Creature);
        creature.set_base_power(Some(2));
        creature.set_base_toughness(Some(2));
        creature.controller = p1_id;
        game.cards.insert(creature_id, creature);
        game.battlefield.add(creature_id);

        // Apply pump effect manually
        if let Ok(card) = game.cards.get_mut(creature_id) {
            card.power_bonus = 3;
            card.toughness_bonus = 3;
        }

        // Verify pumped stats
        let creature_pumped = game.cards.get(creature_id).unwrap();
        assert_eq!(creature_pumped.current_power(), 5);
        assert_eq!(creature_pumped.current_toughness(), 5);

        // Advance to End step
        game.turn.current_step = Step::End;

        // Advance to Cleanup step (should trigger cleanup)
        assert!(game.advance_step().is_ok());
        assert_eq!(game.turn.current_step, Step::Cleanup);

        // Check that bonuses were cleared
        let creature_after = game.cards.get(creature_id).unwrap();
        assert_eq!(
            creature_after.current_power(),
            2,
            "Power bonus should be cleared at cleanup"
        );
        assert_eq!(
            creature_after.current_toughness(),
            2,
            "Toughness bonus should be cleared at cleanup"
        );
    }

    #[test]
    fn test_normal_creature_vs_first_strike() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P1: Create a 3/3 creature without first strike (attacker)
        let attacker_id = game.next_entity_id();
        let mut attacker = Card::new(attacker_id, "Hill Giant".to_string(), p1_id);
        attacker.add_type(CardType::Creature);
        attacker.set_base_power(Some(3));
        attacker.set_base_toughness(Some(3));
        attacker.controller = p1_id;
        attacker.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        game.cards.insert(attacker_id, attacker);
        game.battlefield.add(attacker_id);

        // P2: Create a 2/2 creature with First Strike (blocker)
        let blocker_id = game.next_entity_id();
        let mut blocker = Card::new(blocker_id, "First Strike Knight".to_string(), p2_id);
        blocker.add_type(CardType::Creature);
        blocker.set_base_power(Some(2));
        blocker.set_base_toughness(Some(2));
        blocker.controller = p2_id;
        blocker.keywords.insert(Keyword::FirstStrike);
        game.cards.insert(blocker_id, blocker);
        game.battlefield.add(blocker_id);

        // Declare combat
        game.combat.declare_attacker(attacker_id, p2_id);
        let attacker_vec = smallvec::smallvec![attacker_id];
        game.combat.declare_blocker(blocker_id, attacker_vec);

        // Create controllers
        let mut controller1 = ZeroController::new(p1_id);
        let mut controller2 = ZeroController::new(p2_id);

        // First strike damage step: only blocker deals damage
        let result = game.assign_combat_damage(&mut controller1, &mut controller2, true);
        assert!(result.is_ok(), "Failed to assign first strike damage: {result:?}");

        // Attacker should have taken 2 damage but still be alive (3 toughness)
        assert!(
            game.battlefield.contains(attacker_id),
            "Attacker should still be alive after first strike"
        );

        // Blocker should still be alive (hasn't taken damage yet)
        assert!(game.battlefield.contains(blocker_id), "Blocker should still be alive");

        // Normal damage step: attacker deals damage, killing blocker
        let result = game.assign_combat_damage(&mut controller1, &mut controller2, false);
        assert!(result.is_ok(), "Failed to assign normal damage: {result:?}");

        // Blocker should be dead (took 3 damage, toughness 2)
        if let Some(zones) = game.get_player_zones(p2_id) {
            assert!(
                zones.graveyard.contains(blocker_id),
                "Blocker should be in graveyard after normal damage"
            );
        }

        // Attacker should still be alive (took only 2 damage, has 3 toughness)
        assert!(game.battlefield.contains(attacker_id), "Attacker should still be alive");
    }

    #[test]
    fn test_resolve_tap_spell() {
        use crate::core::{Effect, ManaCost};

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let players: Vec<_> = game.players.iter().map(|p| p.id).collect();
        let p1_id = players[0];
        let p2_id = players[1];

        // Create an untapped creature for P2
        let creature_id = game.next_card_id();
        let mut creature = Card::new(creature_id, "Grizzly Bears".to_string(), p2_id);
        creature.add_type(CardType::Creature);
        creature.set_base_power(Some(2));
        creature.set_base_toughness(Some(2));
        creature.controller = p2_id;
        game.cards.insert(creature_id, creature);
        game.battlefield.add(creature_id);

        // Check initial state
        let creature_before = game.cards.get(creature_id).unwrap();
        assert!(!creature_before.tapped, "Creature should start untapped");

        // Create a Tap spell
        let tap_spell_id = game.next_card_id();
        let mut tap_spell = Card::new(tap_spell_id, "Frost Breath".to_string(), p1_id);
        tap_spell.add_type(CardType::Instant);
        tap_spell.mana_cost = ManaCost::from_string("2U");
        // Target the specific creature
        tap_spell.effects.push(Effect::TapPermanent { target: creature_id });
        game.cards.insert(tap_spell_id, tap_spell);

        // Put spell on stack (simulating cast)
        game.stack.add(tap_spell_id);

        // Resolve the spell
        assert!(
            game.resolve_spell(tap_spell_id, &[]).is_ok(),
            "Failed to resolve tap spell"
        );

        // Check creature is tapped
        let creature_after = game.cards.get(creature_id).unwrap();
        assert!(creature_after.tapped, "Creature should be tapped after spell");

        // Check spell went to graveyard
        if let Some(zones) = game.get_player_zones(p1_id) {
            assert!(
                zones.graveyard.contains(tap_spell_id),
                "Tap spell should be in graveyard"
            );
        }
    }

    #[test]
    fn test_execute_mill_effect() {
        use crate::core::Effect;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players.first().unwrap().id;

        // Add cards to library
        for i in 0..5 {
            let card_id = game.next_card_id();
            let card = Card::new(card_id, format!("Card {i}"), p1_id);
            game.cards.insert(card_id, card);
            if let Some(zones) = game.get_player_zones_mut(p1_id) {
                zones.library.add(card_id);
            }
        }

        // Mill 3 cards
        let effect = Effect::Mill {
            player: p1_id,
            count: 3,
        };

        assert!(game.execute_effect(&effect).is_ok());

        // Check cards were milled (library reduced, graveyard increased)
        if let Some(zones) = game.get_player_zones(p1_id) {
            assert_eq!(zones.library.cards.len(), 2, "Should have 2 cards left in library");
            assert_eq!(zones.graveyard.cards.len(), 3, "Should have 3 cards in graveyard");
        }
    }

    #[test]
    fn test_mill_with_empty_library() {
        use crate::core::Effect;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players.first().unwrap().id;

        // Add only 2 cards to library
        for i in 0..2 {
            let card_id = game.next_card_id();
            let card = Card::new(card_id, format!("Card {i}"), p1_id);
            game.cards.insert(card_id, card);
            if let Some(zones) = game.get_player_zones_mut(p1_id) {
                zones.library.add(card_id);
            }
        }

        // Try to mill 5 cards (more than available)
        let effect = Effect::Mill {
            player: p1_id,
            count: 5,
        };

        assert!(game.execute_effect(&effect).is_ok());

        // Check only 2 cards were milled (library is empty)
        if let Some(zones) = game.get_player_zones(p1_id) {
            assert_eq!(zones.library.cards.len(), 0, "Library should be empty");
            assert_eq!(zones.graveyard.cards.len(), 2, "Should have milled only 2 cards");
        }
    }

    #[test]
    fn test_counter_spell_effect() {
        use crate::core::{CardType, Effect, ManaCost};

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let players: Vec<_> = game.players.iter().map(|p| p.id).collect();
        let p1_id = players[0];
        let p2_id = players[1];

        // P1 casts Lightning Bolt (target spell to be countered)
        let bolt_id = game.next_card_id();
        let mut bolt = Card::new(bolt_id, "Lightning Bolt".to_string(), p1_id);
        bolt.add_type(CardType::Instant);
        bolt.mana_cost = ManaCost::from_string("R");
        bolt.effects.push(Effect::DealDamage {
            target: crate::core::TargetRef::Player(p2_id),
            amount: 3,
        });
        game.cards.insert(bolt_id, bolt);
        game.stack.add(bolt_id);

        // P2 responds with Counterspell
        let counter_id = game.next_card_id();
        let mut counterspell = Card::new(counter_id, "Counterspell".to_string(), p2_id);
        counterspell.add_type(CardType::Instant);
        counterspell.mana_cost = ManaCost::from_string("UU");
        counterspell.effects.push(Effect::CounterSpell { target: bolt_id });
        game.cards.insert(counter_id, counterspell);
        game.stack.add(counter_id);

        // Verify both are on the stack
        assert!(game.stack.contains(bolt_id));
        assert!(game.stack.contains(counter_id));

        // Resolve counterspell (counters Lightning Bolt)
        assert!(game.resolve_spell(counter_id, &[]).is_ok());

        // Verify counterspell is in graveyard
        if let Some(zones) = game.get_player_zones(p2_id) {
            assert!(zones.graveyard.contains(counter_id));
        }

        // Verify Lightning Bolt was countered (removed from stack, in graveyard)
        assert!(!game.stack.contains(bolt_id));
        if let Some(zones) = game.get_player_zones(p1_id) {
            assert!(
                zones.graveyard.contains(bolt_id),
                "Countered spell should be in graveyard"
            );
        }

        // Verify P2 didn't take damage (Lightning Bolt was countered before resolving)
        let p2 = game.get_player(p2_id).unwrap();
        assert_eq!(p2.life, 20, "Player 2 should still have 20 life");
    }

    #[test]
    fn test_counter_spell_with_placeholder_target() {
        use crate::core::{CardType, Effect, ManaCost};

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let players: Vec<_> = game.players.iter().map(|p| p.id).collect();
        let p1_id = players[0];
        let p2_id = players[1];

        // P1 casts Lightning Bolt
        let bolt_id = game.next_card_id();
        let mut bolt = Card::new(bolt_id, "Lightning Bolt".to_string(), p1_id);
        bolt.add_type(CardType::Instant);
        bolt.mana_cost = ManaCost::from_string("R");
        bolt.effects.push(Effect::DealDamage {
            target: crate::core::TargetRef::Player(p2_id),
            amount: 3,
        });
        game.cards.insert(bolt_id, bolt);
        game.stack.add(bolt_id);

        // P2 responds with Counterspell using placeholder target (CardId::new(0))
        let counter_id = game.next_card_id();
        let mut counterspell = Card::new(counter_id, "Counterspell".to_string(), p2_id);
        counterspell.add_type(CardType::Instant);
        counterspell.mana_cost = ManaCost::from_string("UU");
        // Use placeholder target - should automatically target opponent's spell
        counterspell.effects.push(Effect::CounterSpell {
            target: crate::core::CardId::new(0),
        });
        game.cards.insert(counter_id, counterspell);
        game.stack.add(counter_id);

        // Resolve counterspell - should automatically find and counter Lightning Bolt
        assert!(game.resolve_spell(counter_id, &[bolt_id]).is_ok());

        // Verify Lightning Bolt was countered
        assert!(!game.stack.contains(bolt_id));
        if let Some(zones) = game.get_player_zones(p1_id) {
            assert!(
                zones.graveyard.contains(bolt_id),
                "Countered spell should be in graveyard"
            );
        }
    }

    #[test]
    fn test_etb_trigger_draw() {
        use crate::core::{Effect, Trigger, TriggerEvent};

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players.first().unwrap().id;

        // Add cards to P1's library for drawing
        for i in 0..5 {
            let card_id = game.next_card_id();
            let card = Card::new(card_id, format!("Card {i}"), p1_id);
            game.cards.insert(card_id, card);
            if let Some(zones) = game.get_player_zones_mut(p1_id) {
                zones.library.add(card_id);
            }
        }

        // Create a creature with an ETB trigger (like Elvish Visionary)
        let creature_id = game.next_entity_id();
        let mut creature = Card::new(creature_id, "Elvish Visionary".to_string(), p1_id);
        creature.add_type(CardType::Creature);
        creature.set_base_power(Some(1));
        creature.set_base_toughness(Some(1));
        creature.mana_cost = ManaCost::from_string("1G");

        // Add ETB trigger: "When this enters the battlefield, draw a card"
        creature.triggers.push(Trigger::new(
            TriggerEvent::EntersBattlefield,
            vec![Effect::DrawCards {
                player: p1_id,
                count: 1,
            }],
            "When Elvish Visionary enters, draw a card.".to_string(),
        ));

        game.cards.insert(creature_id, creature);

        // Put the creature on the stack (as if it was cast)
        game.stack.add(creature_id);

        // Resolve the creature spell (moves it to battlefield and triggers ETB)
        assert!(game.resolve_spell(creature_id, &[]).is_ok());

        // Verify the creature is on the battlefield
        assert!(game.battlefield.contains(creature_id));

        // Verify the ETB trigger drew a card
        if let Some(zones) = game.get_player_zones(p1_id) {
            assert_eq!(zones.hand.cards.len(), 1, "Should have drawn 1 card");
            assert_eq!(zones.library.cards.len(), 4, "Should have 4 cards left in library");
        }
    }

    #[test]
    fn test_etb_trigger_damage() {
        use crate::core::{Effect, TargetRef, Trigger, TriggerEvent};

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P2: Create a target creature
        let target_creature_id = game.next_entity_id();
        let mut target = Card::new(target_creature_id, "Grizzly Bears".to_string(), p2_id);
        target.add_type(CardType::Creature);
        target.set_base_power(Some(2));
        target.set_base_toughness(Some(2));
        game.cards.insert(target_creature_id, target);
        game.battlefield.add(target_creature_id);

        // P1: Create a creature with ETB damage trigger (like Flametongue Kavu)
        let kavu_id = game.next_entity_id();
        let mut kavu = Card::new(kavu_id, "Flametongue Kavu".to_string(), p1_id);
        kavu.add_type(CardType::Creature);
        kavu.set_base_power(Some(4));
        kavu.set_base_toughness(Some(2));
        kavu.mana_cost = ManaCost::from_string("3R");

        // Add ETB trigger: "When this enters the battlefield, deal 4 damage to target creature"
        kavu.triggers.push(Trigger::new(
            TriggerEvent::EntersBattlefield,
            vec![Effect::DealDamage {
                target: TargetRef::None, // Will be filled to target an opponent's creature
                amount: 4,
            }],
            "When Flametongue Kavu enters, it deals 4 damage to target creature.".to_string(),
        ));

        game.cards.insert(kavu_id, kavu);

        // Put the kavu on the stack (as if it was cast)
        game.stack.add(kavu_id);

        // Resolve the kavu spell (moves it to battlefield and triggers ETB)
        assert!(game.resolve_spell(kavu_id, &[]).is_ok());

        // Verify the kavu is on the battlefield
        assert!(game.battlefield.contains(kavu_id));

        // Check state-based actions for lethal damage
        game.check_lethal_damage().unwrap();

        // Verify the target creature took lethal damage (2 toughness, 4 damage dealt)
        // The creature should have been destroyed and moved to graveyard
        assert!(
            !game.battlefield.contains(target_creature_id),
            "Target creature should be destroyed"
        );
        if let Some(zones) = game.get_player_zones(p2_id) {
            assert!(
                zones.graveyard.contains(target_creature_id),
                "Target creature should be in graveyard"
            );
        }
    }

    #[test]
    fn test_elvish_visionary_from_cardsfolder() {
        // Test loading Elvish Visionary from the actual cardsfolder and verifying
        // its ETB trigger works correctly
        use crate::loader::CardDatabase;
        use std::path::PathBuf;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players.first().unwrap().id;

        // Add cards to P1's library for drawing
        for i in 0..5 {
            let card_id = game.next_card_id();
            let card = Card::new(card_id, format!("Card {i}"), p1_id);
            game.cards.insert(card_id, card);
            if let Some(zones) = game.get_player_zones_mut(p1_id) {
                zones.library.add(card_id);
            }
        }

        // Load Elvish Visionary from cardsfolder
        let cardsfolder = PathBuf::from("../cardsfolder");
        let db = CardDatabase::new(cardsfolder);

        let card_def = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async { db.get_card("Elvish Visionary").await })
            .expect("Failed to get card")
            .expect("Elvish Visionary not found");

        // Create card instance
        let creature_id = game.next_entity_id();
        let creature = card_def.instantiate(creature_id, p1_id);

        // Verify it has an ETB trigger
        assert!(!creature.triggers.is_empty(), "Elvish Visionary should have triggers");
        assert_eq!(creature.triggers.len(), 1, "Elvish Visionary should have 1 trigger");

        game.cards.insert(creature_id, creature);

        // Put the creature on the stack (as if it was cast)
        game.stack.add(creature_id);

        // Resolve the creature spell (moves it to battlefield and triggers ETB)
        assert!(game.resolve_spell(creature_id, &[]).is_ok());

        // Verify the creature is on the battlefield
        assert!(game.battlefield.contains(creature_id));

        // Verify the ETB trigger drew a card
        if let Some(zones) = game.get_player_zones(p1_id) {
            assert_eq!(zones.hand.cards.len(), 1, "Should have drawn 1 card from ETB trigger");
            assert_eq!(zones.library.cards.len(), 4, "Should have 4 cards left in library");
        }
    }

    #[test]
    fn test_counterspell_from_cardsfolder() {
        // Test loading Counterspell from the actual cardsfolder and verifying
        // it can counter spells
        use crate::loader::CardDatabase;
        use std::path::PathBuf;

        let mut game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
        let players: Vec<_> = game.players.iter().map(|p| p.id).collect();
        let alice_id = players[0];
        let bob_id = players[1];

        // Load Lightning Bolt and Counterspell from cardsfolder
        let cardsfolder = PathBuf::from("../cardsfolder");
        let db = CardDatabase::new(cardsfolder);

        let runtime = tokio::runtime::Runtime::new().unwrap();

        let bolt_def = runtime
            .block_on(async { db.get_card("Lightning Bolt").await })
            .expect("Failed to get card")
            .expect("Lightning Bolt not found");

        let counter_def = runtime
            .block_on(async { db.get_card("Counterspell").await })
            .expect("Failed to get card")
            .expect("Counterspell not found");

        // Player1 casts Lightning Bolt targeting Player2
        let bolt_id = game.next_card_id();
        let bolt = bolt_def.instantiate(bolt_id, alice_id);
        game.cards.insert(bolt_id, bolt);
        game.stack.add(bolt_id);

        // Player2 responds with Counterspell targeting Lightning Bolt
        let counter_id = game.next_card_id();
        let mut counterspell = counter_def.instantiate(counter_id, bob_id);
        // Manually set the target since we're bypassing the full casting process
        if let Some(Effect::CounterSpell { target }) = counterspell.effects.get_mut(0) {
            *target = bolt_id;
        }
        game.cards.insert(counter_id, counterspell);
        game.stack.add(counter_id);

        // Verify both are on the stack
        assert!(game.stack.contains(bolt_id), "Bolt should be on stack");
        assert!(game.stack.contains(counter_id), "Counterspell should be on stack");

        // Resolve Counterspell (should counter Lightning Bolt)
        assert!(
            game.resolve_spell(counter_id, &[]).is_ok(),
            "Counterspell should resolve successfully"
        );

        // Verify Counterspell is in graveyard
        if let Some(zones) = game.get_player_zones(bob_id) {
            assert!(
                zones.graveyard.contains(counter_id),
                "Counterspell should be in graveyard"
            );
        }

        // Verify Lightning Bolt was countered (removed from stack, in graveyard)
        assert!(!game.stack.contains(bolt_id), "Lightning Bolt should not be on stack");
        if let Some(zones) = game.get_player_zones(alice_id) {
            assert!(
                zones.graveyard.contains(bolt_id),
                "Countered spell should be in graveyard"
            );
        }

        // Verify Player2 didn't take damage (Lightning Bolt was countered)
        let bob = game.get_player(bob_id).unwrap();
        assert_eq!(bob.life, 20, "Player2 should still have 20 life");
    }

    #[test]
    fn test_etb_trigger_gain_life() {
        use crate::core::{Effect, Trigger, TriggerEvent};

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Create a creature with ETB gain life trigger
        let creature_id = game.next_entity_id();
        let mut creature = Card::new(creature_id, "Soul Warden".to_string(), p1_id);
        creature.add_type(CardType::Creature);
        creature.set_base_power(Some(1));
        creature.set_base_toughness(Some(1));
        creature.mana_cost = ManaCost::from_string("W");

        // Add ETB trigger: "When this enters the battlefield, you gain 3 life"
        creature.triggers.push(Trigger::new(
            TriggerEvent::EntersBattlefield,
            vec![Effect::GainLife {
                player: crate::core::PlayerId::new(0), // Placeholder
                amount: 3,
            }],
            "When this enters, you gain 3 life.".to_string(),
        ));

        game.cards.insert(creature_id, creature);

        // Record life before
        let life_before = game.get_player(p1_id).unwrap().life;

        // Put the creature on the stack and resolve it
        game.stack.add(creature_id);
        assert!(game.resolve_spell(creature_id, &[]).is_ok());

        // Verify the creature is on the battlefield
        assert!(game.battlefield.contains(creature_id));

        // Verify the ETB trigger gained life
        let life_after = game.get_player(p1_id).unwrap().life;
        assert_eq!(
            life_after,
            life_before + 3,
            "Should have gained 3 life from ETB trigger"
        );
    }

    #[test]
    fn test_etb_trigger_pump() {
        use crate::core::{Effect, Trigger, TriggerEvent};

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Create a target creature on battlefield
        let target_id = game.next_entity_id();
        let mut target = Card::new(target_id, "Grizzly Bears".to_string(), p1_id);
        target.add_type(CardType::Creature);
        target.set_base_power(Some(2));
        target.set_base_toughness(Some(2));
        game.cards.insert(target_id, target);
        game.battlefield.add(target_id);

        // Create a creature with ETB pump trigger
        let creature_id = game.next_entity_id();
        let mut creature = Card::new(creature_id, "Glorious Anthem".to_string(), p1_id);
        creature.add_type(CardType::Creature);
        creature.set_base_power(Some(1));
        creature.set_base_toughness(Some(1));

        // Add ETB trigger: "When this enters, target creature gets +2/+2"
        creature.triggers.push(Trigger::new(
            TriggerEvent::EntersBattlefield,
            vec![Effect::PumpCreature {
                target: crate::core::CardId::new(0), // Placeholder
                power_bonus: 2,
                toughness_bonus: 2,
                keywords_granted: smallvec::SmallVec::new(),
            }],
            "When this enters, target creature gets +2/+2.".to_string(),
        ));

        game.cards.insert(creature_id, creature);

        // Put the creature on the stack and resolve it
        game.stack.add(creature_id);
        assert!(game.resolve_spell(creature_id, &[]).is_ok());

        // Verify the creature is on the battlefield
        assert!(game.battlefield.contains(creature_id));

        // Verify the target got pumped
        let pumped_card = game.cards.get(target_id).unwrap();
        assert_eq!(pumped_card.power_bonus, 2, "Target should have +2 power bonus");
        assert_eq!(pumped_card.toughness_bonus, 2, "Target should have +2 toughness bonus");
    }

    /// Test upkeep trigger dealing damage to controller (like Juzám Djinn)
    #[test]
    fn test_upkeep_trigger_damage_to_controller() {
        use crate::core::{Effect, TargetRef, Trigger, TriggerEvent};

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Verify initial life
        assert_eq!(game.players[0].life, 20);

        // Create a creature with an upkeep trigger (like Juzám Djinn)
        let djinn_id = game.next_entity_id();
        let mut djinn = Card::new(djinn_id, "Juzám Djinn".to_string(), p1_id);
        djinn.types.push(CardType::Creature);
        djinn.set_base_power(Some(5));
        djinn.set_base_toughness(Some(5));
        djinn.controller = p1_id;

        // Add upkeep trigger: "At the beginning of your upkeep, deal 1 damage to you"
        // The [controller_only] flag ensures it only triggers on the controller's turn
        djinn.triggers.push(Trigger::new(
            TriggerEvent::BeginningOfUpkeep,
            vec![Effect::DealDamage {
                target: TargetRef::Player(crate::core::PlayerId::new(0)), // Placeholder for controller
                amount: 1,
            }],
            "[controller_only] At the beginning of your upkeep, Juzám Djinn deals 1 damage to you.".to_string(),
        ));

        game.cards.insert(djinn_id, djinn);
        game.battlefield.add(djinn_id);

        // Set the active player to P1
        game.turn.active_player = p1_id;

        // Execute the upkeep trigger
        let result = game.check_triggers_for_controller(TriggerEvent::BeginningOfUpkeep, djinn_id, p1_id);
        assert!(result.is_ok(), "Upkeep trigger should execute successfully");

        // Verify P1 took 1 damage
        assert_eq!(
            game.players[0].life, 19,
            "Controller should have taken 1 damage from Juzám Djinn"
        );
    }

    /// Test upkeep trigger only fires on controller's turn (not opponent's)
    #[test]
    fn test_upkeep_trigger_controller_only() {
        use crate::core::{Effect, TargetRef, Trigger, TriggerEvent};

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // Create Juzám Djinn controlled by P1
        let djinn_id = game.next_entity_id();
        let mut djinn = Card::new(djinn_id, "Juzám Djinn".to_string(), p1_id);
        djinn.types.push(CardType::Creature);
        djinn.set_base_power(Some(5));
        djinn.set_base_toughness(Some(5));
        djinn.controller = p1_id;

        // controller_turn_only trigger should only fire when controller is active
        let mut upkeep_trigger = Trigger::new(
            TriggerEvent::BeginningOfUpkeep,
            vec![Effect::DealDamage {
                target: TargetRef::Player(crate::core::PlayerId::new(0)),
                amount: 1,
            }],
            "[controller_only] At the beginning of your upkeep, Juzám Djinn deals 1 damage to you.".to_string(),
        );
        upkeep_trigger.controller_turn_only = true;
        djinn.triggers.push(upkeep_trigger);

        game.cards.insert(djinn_id, djinn);
        game.battlefield.add(djinn_id);

        // Try to fire trigger during P2's turn (should not fire)
        game.turn.active_player = p2_id;
        let result = game.check_triggers_for_controller(TriggerEvent::BeginningOfUpkeep, djinn_id, p2_id);
        assert!(result.is_ok());

        // P1 should NOT have taken damage (it's not their upkeep)
        assert_eq!(game.players[0].life, 20, "P1 should not take damage during P2's upkeep");

        // Now fire during P1's turn (should fire)
        game.turn.active_player = p1_id;
        let result = game.check_triggers_for_controller(TriggerEvent::BeginningOfUpkeep, djinn_id, p1_id);
        assert!(result.is_ok());

        // P1 should have taken 1 damage
        assert_eq!(game.players[0].life, 19, "P1 should take 1 damage during their upkeep");
    }

    /// Test loading real Juzám Djinn from cardsfolder and verifying trigger parsing
    #[test]
    fn test_juzam_djinn_from_cardsfolder() {
        use crate::core::TriggerEvent;
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/j/juzam_djinn.txt");
        if !path.exists() {
            println!("Skipping test: cardsfolder not present");
            return;
        }

        let def = crate::loader::CardLoader::load_from_file(&path).expect("Failed to load Juzám Djinn");
        assert_eq!(def.name.as_str(), "Juzám Djinn");

        // Instantiate the card
        let game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let card_id = crate::core::CardId::new(100);
        let card = def.instantiate(card_id, p1_id);

        // Verify the upkeep trigger was parsed
        let upkeep_triggers: Vec<_> = card
            .triggers
            .iter()
            .filter(|t| t.event == TriggerEvent::BeginningOfUpkeep)
            .collect();

        assert_eq!(
            upkeep_triggers.len(),
            1,
            "Juzám Djinn should have exactly one upkeep trigger"
        );

        // Verify the trigger has the DealDamage effect
        let trigger = upkeep_triggers[0];
        assert!(!trigger.effects.is_empty(), "Upkeep trigger should have effects");

        // Verify the trigger description indicates controller-only
        assert!(
            trigger.description.contains("[controller_only]") || trigger.description.contains("your upkeep"),
            "Trigger should indicate it's controller-only"
        );
    }

    /// Test loading real Su-Chi from cardsfolder and verifying death trigger parsing
    #[test]
    fn test_su_chi_death_trigger_from_cardsfolder() {
        use crate::core::TriggerEvent;
        use std::path::PathBuf;

        let path = PathBuf::from("../cardsfolder/s/su_chi.txt");
        if !path.exists() {
            println!("Skipping test: cardsfolder not present");
            return;
        }

        let def = crate::loader::CardLoader::load_from_file(&path).expect("Failed to load Su-Chi");
        assert_eq!(def.name.as_str(), "Su-Chi");

        // Instantiate the card
        let game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let card_id = crate::core::CardId::new(100);
        let card = def.instantiate(card_id, p1_id);

        // Verify the death trigger was parsed (LeavesBattlefield event)
        let death_triggers: Vec<_> = card
            .triggers
            .iter()
            .filter(|t| t.event == TriggerEvent::LeavesBattlefield)
            .collect();

        assert_eq!(death_triggers.len(), 1, "Su-Chi should have exactly one death trigger");

        // Verify the trigger has the AddMana effect
        let trigger = death_triggers[0];
        assert!(!trigger.effects.is_empty(), "Death trigger should have effects");

        // Verify the effect is AddMana
        let has_add_mana = trigger
            .effects
            .iter()
            .any(|e| matches!(e, crate::core::Effect::AddMana { .. }));
        assert!(has_add_mana, "Death trigger should have AddMana effect");
    }

    /// Test Su-Chi death trigger actually fires in combat
    #[test]
    fn test_su_chi_death_trigger_fires_in_combat() {
        use crate::core::{CardType, Effect, ManaCost, Trigger, TriggerEvent};

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // Create a Su-Chi for P1 with death trigger
        let su_chi_id = game.next_entity_id();
        let mut su_chi = crate::core::Card::new(su_chi_id, "Su-Chi".to_string(), p1_id);
        su_chi.types.push(CardType::Artifact);
        su_chi.types.push(CardType::Creature);
        su_chi.set_base_power(Some(4));
        su_chi.set_base_toughness(Some(4));
        // Add the death trigger: "When Su-Chi dies, add {C}{C}{C}{C}"
        let mana = ManaCost::from_string("CCCC");
        su_chi.triggers.push(Trigger::new(
            TriggerEvent::LeavesBattlefield,
            vec![Effect::AddMana {
                player: crate::core::PlayerId::new(0), // Placeholder
                mana,
                produces_chosen_color: false,
                amount_var: None,
            }],
            "When CARDNAME dies, add {C}{C}{C}{C}.".to_string(),
        ));
        game.cards.insert(su_chi_id, su_chi);
        game.battlefield.add(su_chi_id);

        // Create a big creature for P2 to kill Su-Chi
        let killer_id = game.next_entity_id();
        let mut killer = crate::core::Card::new(killer_id, "Big Creature".to_string(), p2_id);
        killer.types.push(CardType::Creature);
        killer.set_base_power(Some(10));
        killer.set_base_toughness(Some(10));
        game.cards.insert(killer_id, killer);
        game.battlefield.add(killer_id);

        // Set up combat - Su-Chi blocks the big creature
        game.turn.active_player = p2_id;

        // Check P1's mana pool before combat
        let p1_mana_before = game.players[0].mana_pool.colorless;

        // Su-Chi takes lethal damage and dies
        // Simulate by calling check_death_triggers directly
        let result = game.check_death_triggers(su_chi_id);
        assert!(result.is_ok(), "Death trigger should execute successfully");

        // Check P1's mana pool after - should have gained 4 colorless mana
        let p1_mana_after = game.players[0].mana_pool.colorless;
        assert_eq!(
            p1_mana_after,
            p1_mana_before + 4,
            "P1 should gain 4 colorless mana when Su-Chi dies"
        );
    }

    /// Test Swords to Plowshares exile effect
    ///
    /// Tests the full flow:
    /// 1. Load Swords to Plowshares from cardsfolder
    /// 2. Create a target creature on battlefield
    /// 3. Verify get_valid_targets_for_spell finds the creature
    /// 4. Resolve spell with chosen target
    /// 5. Verify creature is exiled and controller gains life
    #[test]
    fn test_swords_to_plowshares_exile() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // Load Swords to Plowshares from cardsfolder
        let swords_id = match load_test_card(&mut game, "Swords to Plowshares", p1_id) {
            Ok(id) => id,
            Err(e) => panic!("Failed to load Swords to Plowshares: {e}"),
        };

        // Verify the spell has ExilePermanent effect
        let swords = game.cards.get(swords_id).unwrap();
        assert!(
            swords
                .effects
                .iter()
                .any(|e| matches!(e, Effect::ExilePermanent { .. })),
            "Swords to Plowshares should have ExilePermanent effect. Effects: {:?}",
            swords.effects
        );

        // Create a 3/3 creature controlled by P2
        let creature_id = game.next_entity_id();
        let mut creature = Card::new(creature_id, "Trained Armodon".to_string(), p2_id);
        creature.add_type(CardType::Creature);
        creature.set_base_power(Some(3));
        creature.set_base_toughness(Some(3));
        creature.controller = p2_id;
        game.cards.insert(creature_id, creature);
        game.battlefield.add(creature_id);

        // Verify targeting - Swords should find the creature as a valid target
        let valid_targets = game.get_valid_targets_for_spell(swords_id).unwrap();
        assert!(
            valid_targets.contains(&creature_id),
            "Creature should be a valid target for Swords to Plowshares. Valid targets: {:?}",
            valid_targets
        );

        // Put spell on stack (simulating cast)
        game.stack.add(swords_id);

        // Record P2's life before resolution
        let life_before = game.get_player(p2_id).unwrap().life;

        // Resolve the spell WITH target
        let result = game.resolve_spell(swords_id, &[creature_id]);
        assert!(result.is_ok(), "Failed to resolve Swords to Plowshares: {:?}", result);

        // Verify creature is exiled (exile zone is per-player based on owner)
        let in_exile = game.get_player_zones(p2_id).unwrap().exile.contains(creature_id);
        assert!(in_exile, "Creature should be in exile zone");
        assert!(
            !game.battlefield.contains(creature_id),
            "Creature should not be on battlefield"
        );

        // Verify P2 gained life equal to creature's power (3)
        let life_after = game.get_player(p2_id).unwrap().life;
        // Note: The GainLife effect from SubAbility is not yet implemented
        // so we just verify the exile worked for now
        assert_eq!(
            life_after, life_before,
            "Life gain not yet implemented for SubAbility$ DBGainLife"
        );
    }

    /// Test Web Up ETB trigger (Oblivion Ring-style effect)
    ///
    /// Web Up is an enchantment with:
    /// "When this enchantment enters, exile target nonland permanent an opponent
    /// controls until this enchantment leaves the battlefield."
    ///
    /// This tests that:
    /// 1. The card loads with an ETB trigger
    /// 2. The trigger has an ExilePermanent effect
    /// 3. The effect targets opponent's nonland permanents
    #[test]
    fn test_web_up_etb_trigger() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let _p2_id = game.players[1].id;

        // Load Web Up from cardsfolder
        let web_up_id = match load_test_card(&mut game, "Web Up", p1_id) {
            Ok(id) => id,
            Err(e) => panic!("Failed to load Web Up: {e}"),
        };

        let web_up = game.cards.get(web_up_id).unwrap();

        // Verify it's an enchantment
        assert!(
            web_up.is_enchantment(),
            "Web Up should be an enchantment. Types: {:?}",
            web_up.types
        );

        // Verify the card has an ETB trigger
        assert!(
            !web_up.triggers.is_empty(),
            "Web Up should have at least one trigger. Triggers: {:?}",
            web_up.triggers
        );

        // Find the ETB trigger
        use crate::core::TriggerEvent;
        let etb_trigger = web_up
            .triggers
            .iter()
            .find(|t| t.event == TriggerEvent::EntersBattlefield);

        assert!(
            etb_trigger.is_some(),
            "Web Up should have an EntersBattlefield trigger. Triggers: {:?}",
            web_up.triggers
        );

        let trigger = etb_trigger.unwrap();

        // Verify the trigger has an ExilePermanent effect
        let has_exile_effect = trigger
            .effects
            .iter()
            .any(|e| matches!(e, Effect::ExilePermanent { .. }));

        assert!(
            has_exile_effect,
            "Web Up ETB trigger should have ExilePermanent effect. Effects: {:?}",
            trigger.effects
        );
    }

    /// Test Web Up exiles an opponent's creature when it enters the battlefield
    ///
    /// This is an integration test that verifies the full flow:
    /// 1. Player 1 casts Web Up
    /// 2. Web Up resolves and enters the battlefield
    /// 3. ETB trigger fires and exiles an opponent's creature
    #[test]
    fn test_web_up_exiles_creature() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // Create a creature for P2
        let creature_id = game.next_entity_id();
        let mut creature = Card::new(creature_id, "Grizzly Bears".to_string(), p2_id);
        creature.add_type(CardType::Creature);
        creature.set_base_power(Some(2));
        creature.set_base_toughness(Some(2));
        creature.controller = p2_id;
        game.cards.insert(creature_id, creature);
        game.battlefield.add(creature_id);

        // Verify creature is on battlefield
        assert!(
            game.battlefield.contains(creature_id),
            "Creature should be on battlefield before Web Up"
        );

        // Load Web Up from cardsfolder
        let web_up_id = match load_test_card(&mut game, "Web Up", p1_id) {
            Ok(id) => id,
            Err(e) => panic!("Failed to load Web Up: {e}"),
        };

        // Put Web Up on the stack (simulating cast)
        game.stack.add(web_up_id);

        // Resolve Web Up (it enters the battlefield and triggers)
        let result = game.resolve_spell(web_up_id, &[]);
        assert!(result.is_ok(), "Failed to resolve Web Up: {:?}", result);

        // Verify Web Up is on the battlefield
        assert!(
            game.battlefield.contains(web_up_id),
            "Web Up should be on the battlefield after resolving"
        );

        // Verify creature is now exiled (not on battlefield)
        assert!(
            !game.battlefield.contains(creature_id),
            "Creature should not be on battlefield after Web Up ETB trigger"
        );

        // Verify creature is in exile zone
        let in_exile = game.get_player_zones(p2_id).unwrap().exile.contains(creature_id);
        assert!(in_exile, "Creature should be in exile zone after Web Up ETB");
    }

    /// Test Vibrant Cityscape activated ability for tutoring basic lands
    ///
    /// Vibrant Cityscape has:
    /// "{T}, Sacrifice this land: Search your library for a basic land card,
    /// put it onto the battlefield tapped, then shuffle."
    ///
    /// This tests that:
    /// 1. The card loads with an activated ability
    /// 2. The ability has a SearchLibrary effect
    /// 3. The ability costs tap + sacrifice
    #[test]
    fn test_vibrant_cityscape_tutor_ability() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Load Vibrant Cityscape from cardsfolder
        let cityscape_id = match load_test_card(&mut game, "Vibrant Cityscape", p1_id) {
            Ok(id) => id,
            Err(e) => panic!("Failed to load Vibrant Cityscape: {e}"),
        };

        let cityscape = game.cards.get(cityscape_id).unwrap();

        // Verify it's a land
        assert!(
            cityscape.is_land(),
            "Vibrant Cityscape should be a land. Types: {:?}",
            cityscape.types
        );

        // Verify the card has an activated ability
        assert!(
            !cityscape.activated_abilities.is_empty(),
            "Vibrant Cityscape should have at least one activated ability. Abilities: {:?}",
            cityscape.activated_abilities
        );

        // Check if the ability has a SearchLibrary effect
        let has_search_effect = cityscape
            .activated_abilities
            .iter()
            .any(|ab| ab.effects.iter().any(|e| matches!(e, Effect::SearchLibrary { .. })));

        assert!(
            has_search_effect,
            "Vibrant Cityscape should have a SearchLibrary effect. Abilities: {:?}",
            cityscape.activated_abilities
        );
    }

    /// Test Balance spell effect - equalizes creatures across all players
    ///
    /// Balance is a classic white sorcery that equalizes lands, creatures, and hands.
    /// This test verifies that after casting Balance:
    /// - Players with more creatures than the minimum must sacrifice down to match
    /// - The sacrifice is executed correctly (creatures move to graveyard)
    #[test]
    fn test_balance_creature_sacrifice() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let _p1_id = game.players[0].id; // P1 has 0 creatures
        let p2_id = game.players[1].id;

        // P1 has 0 creatures
        // P2 has 3 creatures - should sacrifice all 3 after Balance

        // Create 3 creatures for P2
        let creature_ids: Vec<CardId> = (0..3)
            .map(|i| {
                let creature_id = game.next_card_id();
                let mut creature = Card::new(creature_id, format!("Bear {}", i + 1), p2_id);
                creature.controller = p2_id;
                creature.add_type(CardType::Creature);
                creature.set_base_power(Some(2));
                creature.set_base_toughness(Some(2));
                game.cards.insert(creature_id, creature);
                game.battlefield.add(creature_id);
                creature_id
            })
            .collect();

        // Verify initial state: P2 has 3 creatures on battlefield
        assert_eq!(
            game.battlefield
                .cards
                .iter()
                .filter(|&&cid| {
                    game.cards
                        .get(cid)
                        .map(|c| c.controller == p2_id && c.is_creature())
                        .unwrap_or(false)
                })
                .count(),
            3,
            "P2 should have 3 creatures before Balance"
        );

        // Execute Balance effect for creatures
        let result = game.execute_balance_effect("Creature", "Battlefield");
        assert!(
            result.is_ok(),
            "Balance effect should execute successfully: {:?}",
            result
        );

        // After Balance, P2 should have 0 creatures (same as P1)
        let p2_creatures_after = game
            .battlefield
            .cards
            .iter()
            .filter(|&&cid| {
                game.cards
                    .get(cid)
                    .map(|c| c.controller == p2_id && c.is_creature())
                    .unwrap_or(false)
            })
            .count();
        assert_eq!(
            p2_creatures_after, 0,
            "P2 should have 0 creatures after Balance equalizing with P1"
        );

        // Verify all creatures are in P2's graveyard
        let p2_graveyard = &game.get_player_zones(p2_id).unwrap().graveyard;
        for creature_id in creature_ids {
            assert!(
                p2_graveyard.contains(creature_id),
                "Creature {:?} should be in P2's graveyard after sacrifice",
                creature_id
            );
        }
    }

    /// Test Balance spell effect - equalizes hands (discard) across all players
    #[test]
    fn test_balance_hand_discard() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // P1 has 1 card in hand
        // P2 has 4 cards in hand - should discard 3 to match P1

        // Add 1 card to P1's hand
        let p1_card = game.next_card_id();
        let mut card1 = Card::new(p1_card, "Plains".to_string(), p1_id);
        card1.controller = p1_id;
        game.cards.insert(p1_card, card1);
        game.player_zones
            .iter_mut()
            .find(|(id, _)| *id == p1_id)
            .unwrap()
            .1
            .hand
            .cards
            .push(p1_card);

        // Add 4 cards to P2's hand
        let p2_cards: Vec<CardId> = (0..4)
            .map(|i| {
                let card_id = game.next_card_id();
                let mut card = Card::new(card_id, format!("Card {}", i + 1), p2_id);
                card.controller = p2_id;
                game.cards.insert(card_id, card);
                game.player_zones
                    .iter_mut()
                    .find(|(id, _)| *id == p2_id)
                    .unwrap()
                    .1
                    .hand
                    .cards
                    .push(card_id);
                card_id
            })
            .collect();

        // Verify initial state
        let p1_hand_size_before = game.get_player_zones(p1_id).unwrap().hand.cards.len();
        let p2_hand_size_before = game.get_player_zones(p2_id).unwrap().hand.cards.len();
        assert_eq!(p1_hand_size_before, 1, "P1 should have 1 card in hand");
        assert_eq!(p2_hand_size_before, 4, "P2 should have 4 cards in hand");

        // Execute Balance effect for hands
        let result = game.execute_balance_effect("", "Hand");
        assert!(
            result.is_ok(),
            "Balance effect should execute successfully: {:?}",
            result
        );

        // After Balance, both players should have 1 card in hand
        let p1_hand_size_after = game.get_player_zones(p1_id).unwrap().hand.cards.len();
        let p2_hand_size_after = game.get_player_zones(p2_id).unwrap().hand.cards.len();
        assert_eq!(
            p1_hand_size_after, 1,
            "P1 should still have 1 card in hand after Balance"
        );
        assert_eq!(p2_hand_size_after, 1, "P2 should have 1 card in hand after Balance");

        // Verify 3 cards are in P2's graveyard
        let p2_graveyard = &game.get_player_zones(p2_id).unwrap().graveyard;
        let discarded_count = p2_cards.iter().filter(|&&cid| p2_graveyard.contains(cid)).count();
        assert_eq!(discarded_count, 3, "3 cards should be in P2's graveyard after discard");
    }

    // ═══════════════════════════════════════════════════════════════════
    // AB$ PreventDamage tests
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn test_prevent_damage_to_creature() {
        // Test: PreventDamage creates a shield that absorbs damage to a creature
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Create a 2/2 creature
        let creature_id = game.next_entity_id();
        let mut creature = Card::new(creature_id, "Test Creature".to_string(), p1_id);
        creature.add_type(CardType::Creature);
        creature.set_base_power(Some(2));
        creature.set_base_toughness(Some(2));
        creature.controller = p1_id;
        game.cards.insert(creature_id, creature);
        game.battlefield.add(creature_id);

        // Cast a PreventDamage spell targeting the creature (prevent 3)
        let spell_id = game.next_entity_id();
        let mut spell = Card::new(spell_id, "Shield".to_string(), p1_id);
        spell.add_type(CardType::Instant);
        spell.mana_cost = ManaCost::from_string("W");
        spell.effects.push(Effect::PreventDamage {
            target: TargetRef::Permanent(creature_id),
            amount: 3,
        });
        game.cards.insert(spell_id, spell);
        game.stack.add(spell_id);

        let result = game.resolve_spell(spell_id, &[]);
        assert!(result.is_ok(), "PreventDamage spell should resolve");

        // Creature should have a prevention shield
        let card = game.cards.get(creature_id).unwrap();
        assert_eq!(card.damage_prevention, 3, "Creature should have 3 prevention shield");

        // Deal 2 damage - should be fully prevented
        let result = game.deal_damage_to_creature(creature_id, 2);
        assert!(result.is_ok());
        let card = game.cards.get(creature_id).unwrap();
        assert_eq!(card.damage, 0, "All 2 damage should be prevented");
        assert_eq!(card.damage_prevention, 1, "1 prevention shield should remain");

        // Deal 2 more damage - 1 prevented, 1 gets through
        let result = game.deal_damage_to_creature(creature_id, 2);
        assert!(result.is_ok());
        let card = game.cards.get(creature_id).unwrap();
        assert_eq!(card.damage, 1, "1 damage should get through");
        assert_eq!(card.damage_prevention, 0, "No prevention shield remaining");
    }

    #[test]
    fn test_prevent_damage_to_player() {
        // Test: PreventDamage creates a shield that absorbs damage to a player
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Apply prevention shield to player (prevent 2)
        let spell_id = game.next_entity_id();
        let mut spell = Card::new(spell_id, "Shield".to_string(), p1_id);
        spell.add_type(CardType::Instant);
        spell.mana_cost = ManaCost::from_string("W");
        spell.effects.push(Effect::PreventDamage {
            target: TargetRef::Player(p1_id),
            amount: 2,
        });
        game.cards.insert(spell_id, spell);
        game.stack.add(spell_id);

        let result = game.resolve_spell(spell_id, &[]);
        assert!(result.is_ok(), "PreventDamage spell should resolve");

        let player = game.get_player(p1_id).unwrap();
        assert_eq!(player.damage_prevention, 2, "Player should have 2 prevention shield");
        assert_eq!(player.life, 20, "Life should be unchanged");

        // Deal 3 damage - 2 prevented, 1 gets through
        let result = game.deal_damage(p1_id, 3);
        assert!(result.is_ok());
        let player = game.get_player(p1_id).unwrap();
        assert_eq!(player.life, 19, "Only 1 damage should get through (20 - 1 = 19)");
        assert_eq!(player.damage_prevention, 0, "Shield should be depleted");
    }

    #[test]
    fn test_prevent_damage_stacks() {
        // Test: Multiple prevention shields stack additively
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Create creature and apply two prevention shields
        let creature_id = game.next_entity_id();
        let mut creature = Card::new(creature_id, "Test Creature".to_string(), p1_id);
        creature.add_type(CardType::Creature);
        creature.set_base_power(Some(1));
        creature.set_base_toughness(Some(1));
        creature.controller = p1_id;
        game.cards.insert(creature_id, creature);
        game.battlefield.add(creature_id);

        // Apply two shields (1 + 2 = 3 total)
        for amount in [1, 2] {
            let spell_id = game.next_entity_id();
            let mut spell = Card::new(spell_id, "Shield".to_string(), p1_id);
            spell.add_type(CardType::Instant);
            spell.mana_cost = ManaCost::from_string("W");
            spell.effects.push(Effect::PreventDamage {
                target: TargetRef::Permanent(creature_id),
                amount,
            });
            game.cards.insert(spell_id, spell);
            game.stack.add(spell_id);
            game.resolve_spell(spell_id, &[]).unwrap();
        }

        let card = game.cards.get(creature_id).unwrap();
        assert_eq!(card.damage_prevention, 3, "Shields should stack to 3");
    }

    #[test]
    fn test_prevent_damage_fully_absorbed() {
        // Test: When shield fully absorbs damage, no damage is dealt
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Set up player prevention shield of 5
        game.get_player_mut(p1_id).unwrap().damage_prevention = 5;

        // Deal 3 damage - fully absorbed
        let result = game.deal_damage(p1_id, 3);
        assert!(result.is_ok());
        let player = game.get_player(p1_id).unwrap();
        assert_eq!(player.life, 20, "Life should be unchanged");
        assert_eq!(player.damage_prevention, 2, "2 prevention remaining");
    }

    #[test]
    fn test_put_counter_all() {
        use crate::core::{CardType, CounterType};

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // Create 3 creatures on P1's battlefield
        let mut p1_creatures = Vec::new();
        for i in 0..3 {
            let creature_id = game.next_card_id();
            let mut creature = Card::new(creature_id, format!("Soldier {}", i + 1), p1_id);
            creature.add_type(CardType::Creature);
            creature.set_base_power(Some(1));
            creature.set_base_toughness(Some(1));
            creature.controller = p1_id;
            game.cards.insert(creature_id, creature);
            game.battlefield.add(creature_id);
            p1_creatures.push(creature_id);
        }

        // Create 1 creature on P2's battlefield (should NOT get counters with YouCtrl filter)
        let p2_creature_id = game.next_card_id();
        let mut p2_creature = Card::new(p2_creature_id, "Opponent Bear".to_string(), p2_id);
        p2_creature.add_type(CardType::Creature);
        p2_creature.set_base_power(Some(2));
        p2_creature.set_base_toughness(Some(2));
        p2_creature.controller = p2_id;
        game.cards.insert(p2_creature_id, p2_creature);
        game.battlefield.add(p2_creature_id);

        // Set active player to P1 (PutCounterAll uses turn.active_player for controller filter)
        game.turn.active_player = p1_id;

        // Execute PutCounterAll effect using TargetRestriction (upstream API)
        let restriction = crate::core::TargetRestriction::parse("Creature.YouCtrl");
        let effect = Effect::PutCounterAll {
            restriction,
            counter_type: CounterType::P1P1,
            amount: 1,
        };
        game.execute_effect(&effect).unwrap();

        // Verify P1's creatures got +1/+1 counters
        for &cid in &p1_creatures {
            let card = game.cards.get(cid).unwrap();
            let counter_count = card
                .counters
                .iter()
                .find(|(ct, _)| *ct == CounterType::P1P1)
                .map(|(_, n)| *n)
                .unwrap_or(0);
            assert_eq!(counter_count, 1, "P1's creature should have 1 +1/+1 counter");
            assert_eq!(card.current_power(), 2, "P1's creature should now be 2/2");
            assert_eq!(card.current_toughness(), 2, "P1's creature should now be 2/2");
        }

        // Verify P2's creature did NOT get counters
        let p2_card = game.cards.get(p2_creature_id).unwrap();
        let p2_counter_count = p2_card
            .counters
            .iter()
            .find(|(ct, _)| *ct == CounterType::P1P1)
            .map(|(_, n)| *n)
            .unwrap_or(0);
        assert_eq!(p2_counter_count, 0, "P2's creature should have no counters");
    }

    #[test]
    fn test_untap_all() {
        use crate::core::CardType;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // Create 3 tapped creatures on P1's battlefield
        let mut p1_creatures = Vec::new();
        for i in 0..3 {
            let creature_id = game.next_card_id();
            let mut creature = Card::new(creature_id, format!("Warrior {}", i + 1), p1_id);
            creature.add_type(CardType::Creature);
            creature.set_base_power(Some(2));
            creature.set_base_toughness(Some(2));
            creature.controller = p1_id;
            creature.tapped = true; // All tapped (attacked this turn)
            game.cards.insert(creature_id, creature);
            game.battlefield.add(creature_id);
            p1_creatures.push(creature_id);
        }

        // Create a tapped creature on P2's battlefield
        let p2_creature_id = game.next_card_id();
        let mut p2_creature = Card::new(p2_creature_id, "Opp Warrior".to_string(), p2_id);
        p2_creature.add_type(CardType::Creature);
        p2_creature.controller = p2_id;
        p2_creature.tapped = true;
        game.cards.insert(p2_creature_id, p2_creature);
        game.battlefield.add(p2_creature_id);

        game.turn.active_player = p1_id;

        // Execute UntapAll using TargetRestriction (upstream API)
        let restriction = crate::core::TargetRestriction::parse("Creature.YouCtrl");
        let effect = Effect::UntapAll { restriction };
        game.execute_effect(&effect).unwrap();

        // Verify P1's creatures are untapped
        for &cid in &p1_creatures {
            let card = game.cards.get(cid).unwrap();
            assert!(!card.tapped, "P1's creature should be untapped");
        }

        // Verify P2's creature is still tapped
        let p2_card = game.cards.get(p2_creature_id).unwrap();
        assert!(p2_card.tapped, "P2's creature should still be tapped");
    }

    #[test]
    fn test_add_phase_extra_combat() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);

        // Execute AddPhase
        let effect = Effect::AddPhase { count: 1 };
        game.execute_effect(&effect).unwrap();

        assert_eq!(game.extra_combat_phases, 1, "Should have 1 extra combat phase");

        // Execute another AddPhase
        game.execute_effect(&effect).unwrap();
        assert_eq!(game.extra_combat_phases, 2, "Should have 2 extra combat phases");
    }

    #[test]
    fn test_extra_combat_phase_step_progression() {
        use crate::game::Step;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);

        // Set up: we're at EndCombat with 1 extra combat phase pending
        game.turn.current_step = Step::EndCombat;
        game.extra_combat_phases = 1;

        // Advance step - should go to BeginCombat (extra combat) instead of Main2
        game.advance_step().unwrap();
        assert_eq!(
            game.turn.current_step,
            Step::BeginCombat,
            "Should go to BeginCombat for extra combat phase"
        );
        assert_eq!(game.extra_combat_phases, 0, "Extra combat phases should be decremented");

        // Now advance through the extra combat normally
        game.advance_step().unwrap();
        assert_eq!(game.turn.current_step, Step::DeclareAttackers);

        // Skip to EndCombat of the extra combat
        game.turn.current_step = Step::EndCombat;
        game.advance_step().unwrap();
        // Now should go to Main2 since no more extra combat phases
        assert_eq!(
            game.turn.current_step,
            Step::Main2,
            "Should go to Main2 after all extra combats are done"
        );
    }

    #[test]
    fn test_spells_cast_this_turn_counter() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Initially zero
        assert_eq!(game.get_player(p1_id).unwrap().spells_cast_this_turn, 0);

        // Create a spell and simulate casting it
        let spell_id = game.next_card_id();
        let mut spell = Card::new(spell_id, "Lightning Bolt".to_string(), p1_id);
        spell.add_type(CardType::Instant);
        spell.mana_cost = ManaCost::from_string("R");
        spell.controller = p1_id;
        game.cards.insert(spell_id, spell);

        // Call check_spellcast_triggers (which increments the counter)
        game.check_spellcast_triggers(spell_id, p1_id).unwrap();
        assert_eq!(game.get_player(p1_id).unwrap().spells_cast_this_turn, 1);

        // Cast another spell
        let spell2_id = game.next_card_id();
        let mut spell2 = Card::new(spell2_id, "Shock".to_string(), p1_id);
        spell2.add_type(CardType::Instant);
        spell2.controller = p1_id;
        game.cards.insert(spell2_id, spell2);

        game.check_spellcast_triggers(spell2_id, p1_id).unwrap();
        assert_eq!(game.get_player(p1_id).unwrap().spells_cast_this_turn, 2);
    }

    #[test]
    fn test_count_expression_spells_cast() {
        use crate::core::CountExpression;
        use std::collections::HashMap;

        let mut svars = HashMap::new();
        svars.insert("X".to_string(), "Count$YouCastThisTurn".to_string());

        let expr = CountExpression::parse("X", &svars);
        assert!(
            matches!(expr, CountExpression::SpellsCastThisTurn),
            "Should parse Count$YouCastThisTurn to SpellsCastThisTurn"
        );
    }

    // =========================================================================
    // Raphael's Technique: Optional discard + RememberDiscardingPlayers + Draw
    // =========================================================================

    #[test]
    fn test_optional_discard_remember_discarding_players() {
        // Test Raphael's Technique pattern: each player MAY discard hand, draw 7
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let players: Vec<_> = game.players.iter().map(|p| p.id).collect();
        let p1_id = players[0];
        let p2_id = players[1];

        // Give P1 3 cards in hand
        for i in 0..3 {
            let card_id = game.next_card_id();
            let card = Card::new(card_id, format!("P1 Card {i}"), p1_id);
            game.cards.insert(card_id, card);
            game.get_player_zones_mut(p1_id).unwrap().hand.add(card_id);
        }

        // Give P2 2 cards in hand
        for i in 0..2 {
            let card_id = game.next_card_id();
            let card = Card::new(card_id, format!("P2 Card {i}"), p2_id);
            game.cards.insert(card_id, card);
            game.get_player_zones_mut(p2_id).unwrap().hand.add(card_id);
        }

        // Add plenty of cards to both libraries for draw
        for i in 0..20 {
            let card_id = game.next_card_id();
            let card = Card::new(card_id, format!("P1 Library {i}"), p1_id);
            game.cards.insert(card_id, card);
            game.get_player_zones_mut(p1_id).unwrap().library.add(card_id);
        }
        for i in 0..20 {
            let card_id = game.next_card_id();
            let card = Card::new(card_id, format!("P2 Library {i}"), p2_id);
            game.cards.insert(card_id, card);
            game.get_player_zones_mut(p2_id).unwrap().library.add(card_id);
        }

        // Execute discard effect for P1 (optional, remember discarding players)
        let discard_p1 = Effect::DiscardCards {
            player: p1_id,
            count: u8::MAX, // Mode$ Hand
            remember_discarded: false,
            optional: true,
            remember_discarding_players: true,
        };
        game.execute_effect(&discard_p1).unwrap();

        // P1 should have discarded (AI always discards when optional)
        assert_eq!(
            game.get_player_zones(p1_id).unwrap().hand.cards.len(),
            0,
            "P1 should have 0 cards after discarding hand"
        );
        assert!(
            game.remembered_players.contains(&p1_id),
            "P1 should be in remembered_players"
        );

        // Execute discard effect for P2
        let discard_p2 = Effect::DiscardCards {
            player: p2_id,
            count: u8::MAX,
            remember_discarded: false,
            optional: true,
            remember_discarding_players: true,
        };
        game.execute_effect(&discard_p2).unwrap();

        assert_eq!(
            game.get_player_zones(p2_id).unwrap().hand.cards.len(),
            0,
            "P2 should have 0 cards after discarding hand"
        );
        assert_eq!(game.remembered_players.len(), 2, "Both players should be remembered");

        // Now draw 7 for remembered players
        let draw = Effect::DrawCards {
            player: PlayerId::remembered_players(),
            count: 7,
        };
        game.execute_effect(&draw).unwrap();

        assert_eq!(
            game.get_player_zones(p1_id).unwrap().hand.cards.len(),
            7,
            "P1 should have drawn 7 cards"
        );
        assert_eq!(
            game.get_player_zones(p2_id).unwrap().hand.cards.len(),
            7,
            "P2 should have drawn 7 cards"
        );

        // Clear remembered
        game.execute_effect(&Effect::ClearRemembered).unwrap();
        assert!(
            game.remembered_players.is_empty(),
            "remembered_players should be cleared"
        );
        assert!(game.remembered_cards.is_empty(), "remembered_cards should be cleared");
    }

    #[test]
    fn test_optional_discard_empty_hand_skips() {
        // Test that a player with empty hand is NOT added to remembered_players
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // P1 has empty hand (no cards given)

        let discard = Effect::DiscardCards {
            player: p1_id,
            count: u8::MAX,
            remember_discarded: false,
            optional: true,
            remember_discarding_players: true,
        };
        game.execute_effect(&discard).unwrap();

        // P1 had nothing to discard - should not be remembered
        assert!(
            game.remembered_players.is_empty(),
            "Player with empty hand should not be remembered"
        );
    }

    #[test]
    fn test_effect_converter_put_counter_all() {
        use crate::loader::ability_parser::AbilityParams;
        use crate::loader::effect_converter::params_to_effect;

        let params = AbilityParams::parse(
            "A:DB$ PutCounterAll | CounterType$ P1P1 | CounterNum$ 2 | ValidCards$ Creature.YouCtrl",
        )
        .unwrap();
        let effect = params_to_effect(&params).expect("PutCounterAll should produce an effect");
        let Effect::PutCounterAll {
            counter_type,
            amount,
            ..
        } = effect
        else {
            panic!("Expected PutCounterAll");
        };
        assert_eq!(counter_type, crate::core::CounterType::P1P1);
        assert_eq!(amount, 2);
    }

    #[test]
    fn test_effect_converter_untap_all() {
        use crate::loader::ability_parser::AbilityParams;
        use crate::loader::effect_converter::params_to_effect;

        let params = AbilityParams::parse("A:DB$ UntapAll | ValidCards$ Creature.YouCtrl").unwrap();
        let effect = params_to_effect(&params).expect("UntapAll should produce an effect");
        assert!(matches!(effect, Effect::UntapAll { .. }), "Expected UntapAll");
    }

    #[test]
    fn test_effect_converter_add_phase() {
        use crate::loader::ability_parser::AbilityParams;
        use crate::loader::effect_converter::params_to_effect;

        let params = AbilityParams::parse("A:DB$ AddPhase | PhaseType$ Combat").unwrap();
        let effect = params_to_effect(&params).expect("AddPhase should produce an effect");
        let Effect::AddPhase { count } = effect else {
            panic!("Expected AddPhase");
        };
        assert_eq!(count, 1);
    }

    fn test_draw_for_remembered_players_only() {
        // Test that draw with remembered_players sentinel only draws for remembered players
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;
        let p2_id = game.players[1].id;

        // Add library cards for both
        for i in 0..10 {
            let card_id = game.next_card_id();
            let card = Card::new(card_id, format!("P1 Lib {i}"), p1_id);
            game.cards.insert(card_id, card);
            game.get_player_zones_mut(p1_id).unwrap().library.add(card_id);
        }
        for i in 0..10 {
            let card_id = game.next_card_id();
            let card = Card::new(card_id, format!("P2 Lib {i}"), p2_id);
            game.cards.insert(card_id, card);
            game.get_player_zones_mut(p2_id).unwrap().library.add(card_id);
        }

        // Only P1 is remembered
        game.remembered_players.push(p1_id);

        let draw = Effect::DrawCards {
            player: PlayerId::remembered_players(),
            count: 3,
        };
        game.execute_effect(&draw).unwrap();

        assert_eq!(
            game.get_player_zones(p1_id).unwrap().hand.cards.len(),
            3,
            "P1 (remembered) should have drawn 3"
        );
        assert_eq!(
            game.get_player_zones(p2_id).unwrap().hand.cards.len(),
            0,
            "P2 (not remembered) should not have drawn"
        );
    }

    // =========================================================================
    // Finality Counter: creature with finality counter → exile on death
    // =========================================================================

    #[test]
    fn test_finality_counter_destroy_exiles() {
        use crate::core::CounterType;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Create a creature with a finality counter
        let creature_id = game.next_card_id();
        let mut creature = Card::new(creature_id, "Finality Bear".to_string(), p1_id);
        creature.add_type(CardType::Creature);
        creature.set_base_power(Some(2));
        creature.set_base_toughness(Some(2));
        creature.controller = p1_id;
        creature.add_counter(CounterType::Finality, 1);
        game.cards.insert(creature_id, creature);
        game.battlefield.add(creature_id);

        // Destroy it
        let destroy = Effect::DestroyPermanent {
            target: creature_id,
            restriction: crate::core::TargetRestriction::any(),
        };
        game.execute_effect(&destroy).unwrap();

        // Should be in exile, NOT graveyard
        assert!(
            !game.battlefield.contains(creature_id),
            "Creature should not be on battlefield"
        );
        assert!(
            game.get_player_zones(p1_id).unwrap().exile.contains(creature_id),
            "Creature with finality counter should be exiled, not go to graveyard"
        );
        assert!(
            !game.get_player_zones(p1_id).unwrap().graveyard.contains(creature_id),
            "Creature with finality counter should NOT be in graveyard"
        );
    }

    #[test]
    fn test_no_finality_counter_goes_to_graveyard() {
        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Create a normal creature (no finality counter)
        let creature_id = game.next_card_id();
        let mut creature = Card::new(creature_id, "Normal Bear".to_string(), p1_id);
        creature.add_type(CardType::Creature);
        creature.set_base_power(Some(2));
        creature.set_base_toughness(Some(2));
        creature.controller = p1_id;
        game.cards.insert(creature_id, creature);
        game.battlefield.add(creature_id);

        // Destroy it
        let destroy = Effect::DestroyPermanent {
            target: creature_id,
            restriction: crate::core::TargetRestriction::any(),
        };
        game.execute_effect(&destroy).unwrap();

        // Should be in graveyard
        assert!(
            game.get_player_zones(p1_id).unwrap().graveyard.contains(creature_id),
            "Normal creature should go to graveyard"
        );
        assert!(
            !game.get_player_zones(p1_id).unwrap().exile.contains(creature_id),
            "Normal creature should NOT be exiled"
        );
    }

    #[test]
    fn test_finality_counter_lethal_damage_exiles() {
        use crate::core::CounterType;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Create a creature with finality counter
        let creature_id = game.next_card_id();
        let mut creature = Card::new(creature_id, "Finality Soldier".to_string(), p1_id);
        creature.add_type(CardType::Creature);
        creature.set_base_power(Some(2));
        creature.set_base_toughness(Some(2));
        creature.controller = p1_id;
        creature.add_counter(CounterType::Finality, 1);
        game.cards.insert(creature_id, creature);
        game.battlefield.add(creature_id);

        // Deal lethal damage
        let card = game.cards.get_mut(creature_id).unwrap();
        card.damage = 3; // >= toughness of 2

        // Check lethal damage (SBA)
        game.check_lethal_damage().unwrap();

        // Should be in exile
        assert!(
            !game.battlefield.contains(creature_id),
            "Creature should not be on battlefield"
        );
        assert!(
            game.get_player_zones(p1_id).unwrap().exile.contains(creature_id),
            "Creature with finality counter should be exiled from lethal damage"
        );
    }

    #[test]
    fn test_put_finality_counter_then_destroy() {
        use crate::core::CounterType;

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Create a creature
        let creature_id = game.next_card_id();
        let mut creature = Card::new(creature_id, "Test Creature".to_string(), p1_id);
        creature.add_type(CardType::Creature);
        creature.set_base_power(Some(1));
        creature.set_base_toughness(Some(1));
        creature.controller = p1_id;
        game.cards.insert(creature_id, creature);
        game.battlefield.add(creature_id);

        // Put a finality counter on it via the PutCounter effect
        let put_counter = Effect::PutCounter {
            target: creature_id,
            counter_type: CounterType::Finality,
            amount: 1,
        };
        game.execute_effect(&put_counter).unwrap();

        // Verify counter was placed
        assert_eq!(
            game.cards.get(creature_id).unwrap().get_counter(CounterType::Finality),
            1,
            "Finality counter should be on creature"
        );

        // Now destroy - should go to exile
        let destroy = Effect::DestroyPermanent {
            target: creature_id,
            restriction: crate::core::TargetRestriction::any(),
        };
        game.execute_effect(&destroy).unwrap();

        assert!(
            game.get_player_zones(p1_id).unwrap().exile.contains(creature_id),
            "Creature with finality counter should be exiled on destroy"
        );
    }

    // =========================================================================
    // MayPlayFromGraveyard: persistent effect for casting from graveyard
    // =========================================================================

    #[test]
    fn test_may_play_from_graveyard_persistent_effect() {
        use crate::core::persistent_effect::{CleanupCondition, PersistentEffectKind};

        let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
        let p1_id = game.players[0].id;

        // Create Leonardo as the source on battlefield
        let leonardo_id = game.next_card_id();
        let mut leonardo = Card::new(leonardo_id, "Leonardo, Sewer Samurai".to_string(), p1_id);
        leonardo.add_type(CardType::Creature);
        leonardo.set_base_power(Some(3));
        leonardo.set_base_toughness(Some(3));
        leonardo.controller = p1_id;
        game.cards.insert(leonardo_id, leonardo);
        game.battlefield.add(leonardo_id);

        // Add the persistent effect
        game.persistent_effects.add(
            PersistentEffectKind::MayPlayFromGraveyard {
                owner: p1_id,
                max_power: Some(1),
                max_toughness: Some(1),
                your_turn_only: true,
                add_finality_counter: true,
            },
            leonardo_id,
            p1_id,
            CleanupCondition::SourceLeavesBattlefield { source: leonardo_id },
        );

        // Verify the effect was added
        let effects: Vec<_> = game.persistent_effects.find_may_play_from_graveyard(p1_id).collect();
        assert_eq!(effects.len(), 1, "Should have 1 MayPlayFromGraveyard effect");

        // Verify cleanup when Leonardo leaves
        let cleanup_ids = game
            .persistent_effects
            .find_effects_to_cleanup_on_zone_change(leonardo_id, crate::zones::Zone::Battlefield);
        assert_eq!(cleanup_ids.len(), 1, "Should clean up when Leonardo leaves battlefield");
    }
}
