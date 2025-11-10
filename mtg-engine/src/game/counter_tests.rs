//! Integration tests for counter mechanics

use crate::core::{Card, CardType, CounterType, Effect};
use crate::game::GameState;

#[test]
fn test_put_counter_effect() {
    let mut game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
    let p1_id = game.players[0].id;

    // Create a creature on the battlefield
    let creature_id = game.next_card_id();
    let mut creature = Card::new(creature_id, "Test Creature", p1_id);
    creature.types.push(CardType::Creature);
    creature.set_power(Some(2));
    creature.set_toughness(Some(2));
    game.cards.insert(creature_id, creature);
    game.battlefield.add(creature_id);

    // Execute a PutCounter effect
    let effect = Effect::PutCounter {
        target: creature_id,
        counter_type: CounterType::P1P1,
        amount: 3,
    };

    game.execute_effect(&effect).unwrap();

    // Verify counter was added
    let card = game.cards.get(creature_id).unwrap();
    assert_eq!(card.get_counter(CounterType::P1P1), 3);
    assert_eq!(card.current_power(), 5); // 2 base + 3 counters
    assert_eq!(card.current_toughness(), 5);
}

#[test]
fn test_remove_counter_effect() {
    let mut game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
    let p1_id = game.players[0].id;

    // Create a creature with counters
    let creature_id = game.next_card_id();
    let mut creature = Card::new(creature_id, "Test Creature", p1_id);
    creature.types.push(CardType::Creature);
    creature.set_power(Some(2));
    creature.set_toughness(Some(2));
    creature.add_counter(CounterType::P1P1, 5);
    game.cards.insert(creature_id, creature);
    game.battlefield.add(creature_id);

    // Execute a RemoveCounter effect
    let effect = Effect::RemoveCounter {
        target: creature_id,
        counter_type: CounterType::P1P1,
        amount: 2,
    };

    game.execute_effect(&effect).unwrap();

    // Verify counter was removed
    let card = game.cards.get(creature_id).unwrap();
    assert_eq!(card.get_counter(CounterType::P1P1), 3);
    assert_eq!(card.current_power(), 5); // 2 base + 3 counters
}

#[test]
fn test_counter_undo() {
    let mut game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
    let p1_id = game.players[0].id;

    // Create a creature on the battlefield
    let creature_id = game.next_card_id();
    let mut creature = Card::new(creature_id, "Test Creature", p1_id);
    creature.types.push(CardType::Creature);
    creature.set_power(Some(2));
    creature.set_toughness(Some(2));
    game.cards.insert(creature_id, creature);
    game.battlefield.add(creature_id);

    // Add counters using game state method (which logs for undo)
    game.add_counters(creature_id, CounterType::P1P1, 3).unwrap();

    // Verify counters were added
    {
        let card = game.cards.get(creature_id).unwrap();
        assert_eq!(card.get_counter(CounterType::P1P1), 3);
    }

    // Undo the counter addition
    game.undo().unwrap();

    // Verify counters were removed
    {
        let card = game.cards.get(creature_id).unwrap();
        assert_eq!(card.get_counter(CounterType::P1P1), 0);
    }
}

#[test]
fn test_counter_annihilation_through_effects() {
    let mut game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
    let p1_id = game.players[0].id;

    // Create a creature on the battlefield
    let creature_id = game.next_card_id();
    let mut creature = Card::new(creature_id, "Test Creature", p1_id);
    creature.types.push(CardType::Creature);
    creature.set_power(Some(2));
    creature.set_toughness(Some(2));
    game.cards.insert(creature_id, creature);
    game.battlefield.add(creature_id);

    // Add +1/+1 counters
    let effect1 = Effect::PutCounter {
        target: creature_id,
        counter_type: CounterType::P1P1,
        amount: 5,
    };
    game.execute_effect(&effect1).unwrap();

    // Add -1/-1 counters - should annihilate
    let effect2 = Effect::PutCounter {
        target: creature_id,
        counter_type: CounterType::M1M1,
        amount: 3,
    };
    game.execute_effect(&effect2).unwrap();

    // Verify annihilation occurred
    let card = game.cards.get(creature_id).unwrap();
    assert_eq!(card.get_counter(CounterType::P1P1), 2); // 5 - 3 = 2
    assert_eq!(card.get_counter(CounterType::M1M1), 0); // annihilated
    assert_eq!(card.current_power(), 4); // 2 base + 2 counters
    assert_eq!(card.current_toughness(), 4);
}

#[test]
fn test_multiple_counter_types() {
    let mut game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
    let p1_id = game.players[0].id;

    // Create a permanent on the battlefield
    let permanent_id = game.next_card_id();
    let mut permanent = Card::new(permanent_id, "Test Artifact", p1_id);
    permanent.types.push(CardType::Artifact);
    game.cards.insert(permanent_id, permanent);
    game.battlefield.add(permanent_id);

    // Add different types of counters
    game.add_counters(permanent_id, CounterType::Charge, 3).unwrap();
    game.add_counters(permanent_id, CounterType::P1P1, 2).unwrap();
    game.add_counters(permanent_id, CounterType::Age, 1).unwrap();

    // Verify all counters exist
    let card = game.cards.get(permanent_id).unwrap();
    assert_eq!(card.get_counter(CounterType::Charge), 3);
    assert_eq!(card.get_counter(CounterType::P1P1), 2);
    assert_eq!(card.get_counter(CounterType::Age), 1);
}

#[test]
fn test_remove_counter_undo() {
    let mut game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
    let p1_id = game.players[0].id;

    // Create a creature with counters
    let creature_id = game.next_card_id();
    let mut creature = Card::new(creature_id, "Test Creature", p1_id);
    creature.types.push(CardType::Creature);
    creature.add_counter(CounterType::P1P1, 5);
    game.cards.insert(creature_id, creature);
    game.battlefield.add(creature_id);

    // Remove counters using game state method
    game.remove_counters(creature_id, CounterType::P1P1, 2).unwrap();

    // Verify counters were removed
    {
        let card = game.cards.get(creature_id).unwrap();
        assert_eq!(card.get_counter(CounterType::P1P1), 3);
    }

    // Undo the counter removal
    game.undo().unwrap();

    // Verify counters were restored
    {
        let card = game.cards.get(creature_id).unwrap();
        assert_eq!(card.get_counter(CounterType::P1P1), 5);
    }
}
