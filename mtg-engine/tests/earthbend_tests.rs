//! End-to-end tests for Earthbend mechanic
//!
//! Earthbend is an Avatar set mechanic that:
//! 1. Targets a land you control
//! 2. Makes it a 0/0 creature with haste (still a land)
//! 3. Puts N +1/+1 counters on it
//! 4. Creates a delayed trigger: "When it dies or is exiled, return it to battlefield tapped"
//!
//! These tests verify the implementation of the Effect::Earthbend and the delayed trigger system.

use mtg_engine::core::{Card, CardType, CounterType, Effect, Keyword};
use mtg_engine::game::GameState;
use mtg_engine::zones::Zone;
use smallvec::SmallVec;

/// Helper function to create a basic land card
fn create_land(game: &mut GameState, name: &str, owner: mtg_engine::core::PlayerId) -> mtg_engine::core::CardId {
    let land_id = game.next_card_id();
    let mut land = Card::new(land_id, name, owner);
    land.set_types(SmallVec::from_vec(vec![CardType::Land]));
    land.controller = owner;
    game.cards.insert(land_id, land);
    land_id
}

/// Helper function to create a creature card
fn create_creature(
    game: &mut GameState,
    name: &str,
    owner: mtg_engine::core::PlayerId,
    power: i8,
    toughness: i8,
) -> mtg_engine::core::CardId {
    let creature_id = game.next_card_id();
    let mut creature = Card::new(creature_id, name, owner);
    creature.set_types(SmallVec::from_vec(vec![CardType::Creature]));
    creature.set_base_power(Some(power));
    creature.set_base_toughness(Some(toughness));
    creature.controller = owner;
    game.cards.insert(creature_id, creature);
    creature_id
}

#[test]
fn test_earthbend_basic_effect() {
    // Test that Earthbend turns a land into a 0/0 creature with counters
    let mut game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
    let p1_id = game.players[0].id;

    // Create a land on the battlefield
    let land_id = create_land(&mut game, "Forest", p1_id);
    game.battlefield.add(land_id);

    // Verify initial state: land but not creature
    {
        let land = game.cards.get(land_id).unwrap();
        assert!(land.is_land(), "Should start as a land");
        assert!(!land.is_creature(), "Should not start as a creature");
    }

    // Execute Earthbend with 5 counters
    let effect = Effect::Earthbend {
        target: land_id,
        num_counters: 5,
    };
    game.execute_effect(&effect).expect("Earthbend should succeed");

    // Verify the land is now also a creature
    let land = game.cards.get(land_id).unwrap();
    assert!(land.is_land(), "Should still be a land after earthbend");
    assert!(land.is_creature(), "Should now be a creature after earthbend");

    // Verify it has haste
    assert!(
        land.keywords.contains(Keyword::Haste),
        "Earthbent land should have haste"
    );

    // Verify counters
    assert_eq!(land.get_counter(CounterType::P1P1), 5, "Should have 5 +1/+1 counters");

    // Verify power/toughness: base 0/0 + 5 counters = 5/5
    assert_eq!(land.current_power(), 5, "Power should be 5 (0 base + 5 counters)");
    assert_eq!(
        land.current_toughness(),
        5,
        "Toughness should be 5 (0 base + 5 counters)"
    );
}

#[test]
fn test_earthbend_different_counter_amounts() {
    // Test Earthbend with different numbers of counters
    let mut game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
    let p1_id = game.players[0].id;

    // Test with 1 counter
    let land1_id = create_land(&mut game, "Plains", p1_id);
    game.battlefield.add(land1_id);

    let effect1 = Effect::Earthbend {
        target: land1_id,
        num_counters: 1,
    };
    game.execute_effect(&effect1).expect("Earthbend should succeed");

    let land1 = game.cards.get(land1_id).unwrap();
    assert_eq!(land1.current_power(), 1);
    assert_eq!(land1.current_toughness(), 1);

    // Test with 8 counters (max typical Avatar Kyoshi)
    let land2_id = create_land(&mut game, "Mountain", p1_id);
    game.battlefield.add(land2_id);

    let effect2 = Effect::Earthbend {
        target: land2_id,
        num_counters: 8,
    };
    game.execute_effect(&effect2).expect("Earthbend should succeed");

    let land2 = game.cards.get(land2_id).unwrap();
    assert_eq!(land2.current_power(), 8);
    assert_eq!(land2.current_toughness(), 8);
}

#[test]
fn test_earthbend_registers_delayed_trigger() {
    // Test that Earthbend creates a delayed trigger for return-to-battlefield
    let mut game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
    let p1_id = game.players[0].id;

    // Create a land on the battlefield
    let land_id = create_land(&mut game, "Forest", p1_id);
    game.battlefield.add(land_id);

    // Initially, no delayed triggers
    assert_eq!(
        game.delayed_triggers.all().len(),
        0,
        "Should start with no delayed triggers"
    );

    // Execute Earthbend
    let effect = Effect::Earthbend {
        target: land_id,
        num_counters: 3,
    };
    game.execute_effect(&effect).expect("Earthbend should succeed");

    // Verify a delayed trigger was registered
    assert_eq!(
        game.delayed_triggers.all().len(),
        1,
        "Should have one delayed trigger after earthbend"
    );

    // Verify the trigger is for the earthbent land
    let trigger = &game.delayed_triggers.all()[0];
    assert_eq!(trigger.tracked_card, land_id, "Trigger should track the earthbent land");
}

#[test]
fn test_earthbend_delayed_trigger_fires_on_death() {
    // Test that when an earthbent land dies, it returns to battlefield tapped
    let mut game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
    let p1_id = game.players[0].id;

    // Create a land on the battlefield
    let land_id = create_land(&mut game, "Forest", p1_id);
    game.battlefield.add(land_id);

    // Earthbend it
    let effect = Effect::Earthbend {
        target: land_id,
        num_counters: 3,
    };
    game.execute_effect(&effect).expect("Earthbend should succeed");

    // Verify land is on battlefield
    assert!(game.battlefield.contains(land_id), "Land should be on battlefield");

    // Move the land to graveyard (simulating death)
    game.move_card(land_id, Zone::Battlefield, Zone::Graveyard, p1_id)
        .expect("Move to graveyard should succeed");

    // The delayed trigger should have fired, returning the land to battlefield
    assert!(
        game.battlefield.contains(land_id),
        "Land should return to battlefield after dying"
    );

    // Verify it returned tapped
    let land = game.cards.get(land_id).unwrap();
    assert!(land.tapped, "Returned land should be tapped");

    // Delayed trigger should be consumed
    assert_eq!(
        game.delayed_triggers.all().len(),
        0,
        "Delayed trigger should be removed after firing"
    );
}

#[test]
fn test_earthbend_delayed_trigger_fires_on_exile() {
    // Test that when an earthbent land is exiled, it returns to battlefield tapped
    let mut game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
    let p1_id = game.players[0].id;

    // Create a land on the battlefield
    let land_id = create_land(&mut game, "Island", p1_id);
    game.battlefield.add(land_id);

    // Earthbend it
    let effect = Effect::Earthbend {
        target: land_id,
        num_counters: 4,
    };
    game.execute_effect(&effect).expect("Earthbend should succeed");

    // Verify land is on battlefield
    assert!(game.battlefield.contains(land_id), "Land should be on battlefield");

    // Move the land to exile
    game.move_card(land_id, Zone::Battlefield, Zone::Exile, p1_id)
        .expect("Move to exile should succeed");

    // The delayed trigger should have fired, returning the land to battlefield
    assert!(
        game.battlefield.contains(land_id),
        "Land should return to battlefield after being exiled"
    );

    // Verify it returned tapped
    let land = game.cards.get(land_id).unwrap();
    assert!(land.tapped, "Returned land should be tapped");
}

#[test]
fn test_earthbend_target_must_be_land() {
    // Test that Earthbend fails if target is not a land
    let mut game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
    let p1_id = game.players[0].id;

    // Create a creature (not a land)
    let creature_id = create_creature(&mut game, "Grizzly Bears", p1_id, 2, 2);
    game.battlefield.add(creature_id);

    // Try to earthbend a creature - should fail
    let effect = Effect::Earthbend {
        target: creature_id,
        num_counters: 3,
    };
    let result = game.execute_effect(&effect);

    assert!(result.is_err(), "Earthbend should fail on non-land target");
}

#[test]
fn test_earthbend_preserves_land_type() {
    // Test that the earthbent land keeps its original types/subtypes
    let mut game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
    let p1_id = game.players[0].id;

    // Create a land on the battlefield
    let land_id = create_land(&mut game, "Forest", p1_id);
    game.battlefield.add(land_id);

    // Execute Earthbend
    let effect = Effect::Earthbend {
        target: land_id,
        num_counters: 5,
    };
    game.execute_effect(&effect).expect("Earthbend should succeed");

    // Verify the land has both Land AND Creature types
    let land = game.cards.get(land_id).unwrap();
    assert!(land.is_type(&CardType::Land), "Should still have Land type");
    assert!(land.is_type(&CardType::Creature), "Should now have Creature type");

    // The cache should be updated too
    assert!(land.is_land(), "is_land() should return true");
    assert!(land.is_creature(), "is_creature() should return true");
}

#[test]
fn test_earthbend_with_placeholder_target_fizzles() {
    // Test that Earthbend with placeholder target (0) fizzles gracefully
    let mut game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);

    // Execute Earthbend with placeholder target
    let effect = Effect::Earthbend {
        target: mtg_engine::core::CardId::new(0),
        num_counters: 5,
    };
    let result = game.execute_effect(&effect);

    // Should succeed (fizzle = OK result, just no effect)
    assert!(result.is_ok(), "Earthbend with placeholder should fizzle gracefully");

    // No delayed triggers should be created
    assert_eq!(
        game.delayed_triggers.all().len(),
        0,
        "No delayed trigger should be created for fizzled earthbend"
    );
}

#[test]
fn test_multiple_earthbends_multiple_triggers() {
    // Test that earthbending multiple lands creates multiple independent triggers
    let mut game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
    let p1_id = game.players[0].id;

    // Create two lands
    let land1_id = create_land(&mut game, "Forest", p1_id);
    let land2_id = create_land(&mut game, "Mountain", p1_id);
    game.battlefield.add(land1_id);
    game.battlefield.add(land2_id);

    // Earthbend both
    game.execute_effect(&Effect::Earthbend {
        target: land1_id,
        num_counters: 3,
    })
    .expect("Earthbend should succeed");

    game.execute_effect(&Effect::Earthbend {
        target: land2_id,
        num_counters: 5,
    })
    .expect("Earthbend should succeed");

    // Should have two delayed triggers
    assert_eq!(
        game.delayed_triggers.all().len(),
        2,
        "Should have two delayed triggers for two earthbent lands"
    );

    // Each trigger should track a different land
    let triggers = game.delayed_triggers.all();
    let tracked_cards: Vec<_> = triggers.iter().map(|t| t.tracked_card).collect();
    assert!(tracked_cards.contains(&land1_id), "First land should have a trigger");
    assert!(tracked_cards.contains(&land2_id), "Second land should have a trigger");
}
