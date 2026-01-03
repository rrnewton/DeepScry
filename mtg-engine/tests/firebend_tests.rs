//! End-to-end tests for Firebend mechanic
//!
//! Firebend is an Avatar set mechanic that:
//! 1. Triggers when a creature attacks
//! 2. Adds N red mana to the controller's combat mana pool
//! 3. The mana lasts until end of combat (cleared in end_combat_step)
//!
//! These tests verify the implementation of Effect::Firebend and combat mana pools.

use mtg_forge_rs::core::{Card, CardType, Effect, PlayerId};
use mtg_forge_rs::game::GameState;
use smallvec::SmallVec;

/// Helper function to create a creature card
fn create_creature(
    game: &mut GameState,
    name: &str,
    owner: PlayerId,
    power: i8,
    toughness: i8,
) -> mtg_forge_rs::core::CardId {
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
fn test_firebend_basic_effect() {
    // Test that Firebend adds red mana to combat mana pool
    let mut game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
    let p1_id = game.players[0].id;

    // Verify initial state: no combat mana
    assert_eq!(
        game.players[0].combat_mana_pool.red, 0,
        "Should start with no combat mana"
    );

    // Execute Firebend with 3 red mana
    let effect = Effect::Firebend {
        controller: p1_id,
        amount: 3,
    };
    game.execute_effect(&effect).expect("Firebend should succeed");

    // Verify combat mana was added
    assert_eq!(
        game.players[0].combat_mana_pool.red, 3,
        "Should have 3 red combat mana after firebend"
    );

    // Verify regular mana pool is unchanged
    assert_eq!(
        game.players[0].mana_pool.red, 0,
        "Regular mana pool should be unchanged"
    );
}

#[test]
fn test_firebend_multiple_adds_stack() {
    // Test that multiple Firebends stack
    let mut game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
    let p1_id = game.players[0].id;

    // Execute Firebend twice
    let effect1 = Effect::Firebend {
        controller: p1_id,
        amount: 2,
    };
    game.execute_effect(&effect1).expect("Firebend 1 should succeed");

    let effect2 = Effect::Firebend {
        controller: p1_id,
        amount: 3,
    };
    game.execute_effect(&effect2).expect("Firebend 2 should succeed");

    // Verify combat mana accumulated
    assert_eq!(
        game.players[0].combat_mana_pool.red, 5,
        "Should have 5 red combat mana (2 + 3)"
    );
}

#[test]
fn test_firebend_different_players() {
    // Test that Firebend goes to the correct player
    let mut game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
    let p1_id = game.players[0].id;
    let p2_id = game.players[1].id;

    // Firebend for P1
    let effect1 = Effect::Firebend {
        controller: p1_id,
        amount: 2,
    };
    game.execute_effect(&effect1).expect("Firebend P1 should succeed");

    // Firebend for P2
    let effect2 = Effect::Firebend {
        controller: p2_id,
        amount: 4,
    };
    game.execute_effect(&effect2).expect("Firebend P2 should succeed");

    // Verify each player got their mana
    assert_eq!(
        game.players[0].combat_mana_pool.red, 2,
        "P1 should have 2 red combat mana"
    );
    assert_eq!(
        game.players[1].combat_mana_pool.red, 4,
        "P2 should have 4 red combat mana"
    );
}

#[test]
fn test_combat_mana_pool_clear() {
    // Test that combat mana pool can be cleared
    let mut game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
    let p1_id = game.players[0].id;

    // Add combat mana
    let effect = Effect::Firebend {
        controller: p1_id,
        amount: 5,
    };
    game.execute_effect(&effect).expect("Firebend should succeed");

    assert_eq!(game.players[0].combat_mana_pool.red, 5);

    // Clear combat mana pool
    game.players[0].empty_combat_mana_pool();

    assert_eq!(game.players[0].combat_mana_pool.red, 0, "Combat mana should be cleared");
    assert_eq!(
        game.players[0].combat_mana_pool.total(),
        0,
        "Total combat mana should be 0"
    );
}

#[test]
fn test_total_available_mana() {
    // Test that total_available_mana combines regular and combat mana
    let mut game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
    let p1_id = game.players[0].id;

    // Add some regular mana
    game.players[0].mana_pool.red = 2;
    game.players[0].mana_pool.green = 1;

    // Add combat mana via Firebend
    let effect = Effect::Firebend {
        controller: p1_id,
        amount: 3,
    };
    game.execute_effect(&effect).expect("Firebend should succeed");

    // Check total available
    let total = game.players[0].total_available_mana();
    assert_eq!(total.red, 5, "Total red should be 2 regular + 3 combat = 5");
    assert_eq!(total.green, 1, "Total green should be 1 (regular only)");
    assert_eq!(total.total(), 6, "Total mana should be 6");
}

#[test]
fn test_firebend_zero_amount() {
    // Test Firebend with 0 amount (used as sentinel for "use creature power")
    let mut game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
    let p1_id = game.players[0].id;

    // Execute Firebend with 0 (should add nothing when executed directly)
    let effect = Effect::Firebend {
        controller: p1_id,
        amount: 0,
    };
    game.execute_effect(&effect).expect("Firebend 0 should succeed");

    // No mana should be added (0 iterations)
    assert_eq!(game.players[0].combat_mana_pool.red, 0, "Firebend 0 adds no mana");
}

#[test]
fn test_firebend_attack_trigger_keyword() {
    // Test that a card with Firebending keyword has an attack trigger
    use mtg_forge_rs::core::{Keyword, KeywordArgs};

    let mut game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
    let p1_id = game.players[0].id;

    // Create a creature with Firebending keyword
    let creature_id = create_creature(&mut game, "Fire Nation Cadets", p1_id, 2, 2);

    // Add Firebending keyword with amount 1
    {
        let creature = game.cards.get_mut(creature_id).unwrap();
        creature.keywords.insert_complex(KeywordArgs::Firebending { amount: 1 });
    }

    // The keyword itself doesn't auto-create triggers in tests (that happens in card loading)
    // So we manually verify the keyword was added
    {
        let creature = game.cards.get(creature_id).unwrap();
        assert!(
            creature.keywords.contains(Keyword::Firebending),
            "Creature should have Firebending keyword"
        );

        // Verify keyword args
        if let Some(KeywordArgs::Firebending { amount }) = creature.keywords.get_args(Keyword::Firebending) {
            assert_eq!(*amount, 1, "Firebending amount should be 1");
        } else {
            panic!("Should have Firebending args");
        }
    }
}

#[test]
fn test_firebend_attack_trigger_execution() {
    // Test that check_attack_triggers executes Firebend effects
    use mtg_forge_rs::core::{Effect, Trigger, TriggerEvent};

    let mut game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
    let p1_id = game.players[0].id;

    // Create a creature with an attack trigger that firebends
    let creature_id = create_creature(&mut game, "Firebender", p1_id, 3, 3);
    game.battlefield.add(creature_id);

    // Add a Firebend attack trigger manually
    {
        let creature = game.cards.get_mut(creature_id).unwrap();
        creature.triggers.push(Trigger::new(
            TriggerEvent::Attacks,
            vec![Effect::Firebend {
                controller: PlayerId::new(0), // Placeholder - will be resolved
                amount: 2,
            }],
            "Firebending 2".to_string(),
        ));
    }

    // Verify initial state
    assert_eq!(game.players[0].combat_mana_pool.red, 0);

    // Check attack triggers (simulates what happens when creature attacks)
    game.check_attack_triggers(creature_id, p1_id)
        .expect("Attack triggers should succeed");

    // Verify Firebend executed
    assert_eq!(
        game.players[0].combat_mana_pool.red, 2,
        "Firebend should have added 2 red combat mana"
    );
}

#[test]
fn test_firebend_attack_trigger_with_power() {
    // Test Firebending X where X is creature's power
    use mtg_forge_rs::core::{Effect, Trigger, TriggerEvent};

    let mut game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
    let p1_id = game.players[0].id;

    // Create a 4/4 creature with Firebending X trigger
    let creature_id = create_creature(&mut game, "Firebending Student", p1_id, 4, 4);
    game.battlefield.add(creature_id);

    // Add a Firebend attack trigger with amount=0 (meaning use creature's power)
    {
        let creature = game.cards.get_mut(creature_id).unwrap();
        creature.triggers.push(Trigger::new(
            TriggerEvent::Attacks,
            vec![Effect::Firebend {
                controller: PlayerId::new(0), // Placeholder
                amount: 0,                    // 0 = use creature's power
            }],
            "Firebending X, where X is this creature's power".to_string(),
        ));
    }

    // Verify initial state
    assert_eq!(game.players[0].combat_mana_pool.red, 0);

    // Check attack triggers - should use power (4)
    game.check_attack_triggers(creature_id, p1_id)
        .expect("Attack triggers should succeed");

    // Verify Firebend used creature's power
    assert_eq!(
        game.players[0].combat_mana_pool.red, 4,
        "Firebend X should have added 4 red combat mana (creature's power)"
    );
}
