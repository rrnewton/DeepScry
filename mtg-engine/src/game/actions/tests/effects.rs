use crate::core::{Card, CardId, CardType, Effect, Keyword, ManaCost, PlayerId};
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
        creature.types.push(CardType::Creature);
        creature.set_power(Some(2));
        creature.set_toughness(Some(2));
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
        pump_spell.types.push(CardType::Instant);
        pump_spell.mana_cost = ManaCost::from_string("G");
        // Target the creature we created
        pump_spell.effects.push(Effect::PumpCreature {
            target: creature_id,
            power_bonus: 3,
            toughness_bonus: 3,
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
        creature.types.push(CardType::Creature);
        creature.set_power(Some(2));
        creature.set_toughness(Some(2));
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
        attacker.types.push(CardType::Creature);
        attacker.set_power(Some(3));
        attacker.set_toughness(Some(3));
        attacker.controller = p1_id;
        attacker.turn_entered_battlefield = Some(game.turn.turn_number - 1);
        game.cards.insert(attacker_id, attacker);
        game.battlefield.add(attacker_id);

        // P2: Create a 2/2 creature with First Strike (blocker)
        let blocker_id = game.next_entity_id();
        let mut blocker = Card::new(blocker_id, "First Strike Knight".to_string(), p2_id);
        blocker.types.push(CardType::Creature);
        blocker.set_power(Some(2));
        blocker.set_toughness(Some(2));
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
        creature.types.push(CardType::Creature);
        creature.set_power(Some(2));
        creature.set_toughness(Some(2));
        creature.controller = p2_id;
        game.cards.insert(creature_id, creature);
        game.battlefield.add(creature_id);

        // Check initial state
        let creature_before = game.cards.get(creature_id).unwrap();
        assert!(!creature_before.tapped, "Creature should start untapped");

        // Create a Tap spell
        let tap_spell_id = game.next_card_id();
        let mut tap_spell = Card::new(tap_spell_id, "Frost Breath".to_string(), p1_id);
        tap_spell.types.push(CardType::Instant);
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
        bolt.types.push(CardType::Instant);
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
        counterspell.types.push(CardType::Instant);
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
        bolt.types.push(CardType::Instant);
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
        counterspell.types.push(CardType::Instant);
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
        creature.types.push(CardType::Creature);
        creature.set_power(Some(1));
        creature.set_toughness(Some(1));
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
        target.types.push(CardType::Creature);
        target.set_power(Some(2));
        target.set_toughness(Some(2));
        game.cards.insert(target_creature_id, target);
        game.battlefield.add(target_creature_id);

        // P1: Create a creature with ETB damage trigger (like Flametongue Kavu)
        let kavu_id = game.next_entity_id();
        let mut kavu = Card::new(kavu_id, "Flametongue Kavu".to_string(), p1_id);
        kavu.types.push(CardType::Creature);
        kavu.set_power(Some(4));
        kavu.set_toughness(Some(2));
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
        creature.types.push(CardType::Creature);
        creature.set_power(Some(1));
        creature.set_toughness(Some(1));
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
        target.types.push(CardType::Creature);
        target.set_power(Some(2));
        target.set_toughness(Some(2));
        game.cards.insert(target_id, target);
        game.battlefield.add(target_id);

        // Create a creature with ETB pump trigger
        let creature_id = game.next_entity_id();
        let mut creature = Card::new(creature_id, "Glorious Anthem".to_string(), p1_id);
        creature.types.push(CardType::Creature);
        creature.set_power(Some(1));
        creature.set_toughness(Some(1));

        // Add ETB trigger: "When this enters, target creature gets +2/+2"
        creature.triggers.push(Trigger::new(
            TriggerEvent::EntersBattlefield,
            vec![Effect::PumpCreature {
                target: crate::core::CardId::new(0), // Placeholder
                power_bonus: 2,
                toughness_bonus: 2,
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
}
