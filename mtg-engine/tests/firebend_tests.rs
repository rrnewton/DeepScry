//! End-to-end tests for Firebend mechanic
//!
//! Firebend is an Avatar set mechanic that:
//! 1. Triggers when a creature attacks
//! 2. Adds N red mana to the controller's combat mana pool
//! 3. The mana lasts until end of combat (cleared in end_combat_step)
//!
//! These tests verify the implementation of Effect::Firebend and combat mana pools.

use mtg_engine::core::{Card, CardType, Effect, PlayerId};
use mtg_engine::game::GameState;
use smallvec::SmallVec;

/// Helper function to create a creature card
fn create_creature(
    game: &mut GameState,
    name: &str,
    owner: PlayerId,
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

/// Helper to get combat mana red count (returns 0 if None)
fn combat_red(player: &mtg_engine::core::Player) -> u8 {
    player.combat_mana_pool.as_ref().map_or(0, |p| p.red)
}

/// Helper to get combat mana total (returns 0 if None)
fn combat_total(player: &mtg_engine::core::Player) -> u8 {
    player.combat_mana_pool.as_ref().map_or(0, |p| p.total())
}

#[test]
fn test_firebend_basic_effect() {
    // Test that Firebend adds red mana to combat mana pool
    let mut game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);
    let p1_id = game.players[0].id;

    // Verify initial state: no combat mana (Option is None)
    assert!(
        game.players[0].combat_mana_pool.is_none(),
        "Should start with no combat mana pool allocated"
    );

    // Execute Firebend with 3 red mana
    let effect = Effect::Firebend {
        controller: p1_id,
        amount: 3,
    };
    game.execute_effect(&effect).expect("Firebend should succeed");

    // Verify combat mana was added (Option is now Some)
    assert!(
        game.players[0].combat_mana_pool.is_some(),
        "Combat mana pool should be allocated after firebend"
    );
    assert_eq!(
        combat_red(&game.players[0]),
        3,
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
    assert_eq!(combat_red(&game.players[0]), 5, "Should have 5 red combat mana (2 + 3)");
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
    assert_eq!(combat_red(&game.players[0]), 2, "P1 should have 2 red combat mana");
    assert_eq!(combat_red(&game.players[1]), 4, "P2 should have 4 red combat mana");
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

    assert_eq!(combat_red(&game.players[0]), 5);
    assert!(game.players[0].combat_mana_pool.is_some());

    // Clear combat mana pool
    game.players[0].empty_combat_mana_pool();

    // After clearing, Option should be None (deallocated)
    assert!(
        game.players[0].combat_mana_pool.is_none(),
        "Combat mana pool should be deallocated after clear"
    );
    assert_eq!(combat_total(&game.players[0]), 0, "Total combat mana should be 0");
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
fn test_total_available_mana_no_combat() {
    // Test that total_available_mana works when no combat mana (fast path)
    let mut game = GameState::new_two_player("Player1".to_string(), "Player2".to_string(), 20);

    // Add some regular mana only
    game.players[0].mana_pool.red = 2;
    game.players[0].mana_pool.green = 1;

    // No combat mana - should return regular pool directly
    assert!(game.players[0].combat_mana_pool.is_none());

    let total = game.players[0].total_available_mana();
    assert_eq!(total.red, 2, "Total red should be 2 (regular only)");
    assert_eq!(total.green, 1, "Total green should be 1 (regular only)");
    assert_eq!(total.total(), 3, "Total mana should be 3");
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

    // No mana should be added (0 iterations), pool stays None
    assert!(
        game.players[0].combat_mana_pool.is_none(),
        "Firebend 0 should not allocate combat mana pool"
    );
}

#[test]
fn test_firebend_attack_trigger_keyword() {
    // Test that a card with Firebending keyword has an attack trigger
    use mtg_engine::core::{Keyword, KeywordArgs};

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
    use mtg_engine::core::{Effect, Trigger, TriggerEvent};

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

    // Verify initial state - no combat mana allocated
    assert!(game.players[0].combat_mana_pool.is_none());

    // Check attack triggers (simulates what happens when creature attacks)
    game.check_attack_triggers(creature_id, p1_id)
        .expect("Attack triggers should succeed");

    // Verify Firebend executed and allocated combat mana
    assert!(game.players[0].combat_mana_pool.is_some());
    assert_eq!(
        combat_red(&game.players[0]),
        2,
        "Firebend should have added 2 red combat mana"
    );
}

#[test]
fn test_firebend_attack_trigger_with_power() {
    // Test Firebending X where X is creature's power
    use mtg_engine::core::{Effect, Trigger, TriggerEvent};

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

    // Verify initial state - no combat mana
    assert!(game.players[0].combat_mana_pool.is_none());

    // Check attack triggers - should use power (4)
    game.check_attack_triggers(creature_id, p1_id)
        .expect("Attack triggers should succeed");

    // Verify Firebend used creature's power
    assert_eq!(
        combat_red(&game.players[0]),
        4,
        "Firebend X should have added 4 red combat mana (creature's power)"
    );
}

/// Full e2e test: Attack with Firebending creature, generate combat mana, cast a spell with it
///
/// This test demonstrates the complete Firebending workflow:
/// 1. Declare a creature with Firebending as attacker
/// 2. Firebending triggers and adds red mana to combat mana pool
/// 3. Player casts Lightning Bolt (R cost) using the combat mana
/// 4. Combat mana is spent, spell resolves
#[test]
fn test_firebend_e2e_attack_and_cast_spell() {
    use mtg_engine::core::{Card, CardType, Effect, ManaCost, Trigger, TriggerEvent};

    let mut game = GameState::new_two_player("Alice".to_string(), "Bob".to_string(), 20);
    let alice = game.players[0].id;
    let bob = game.players[1].id;

    // Create a 2/2 creature with Firebending 2
    let firebender_id = create_creature(&mut game, "Fire Nation Cadet", alice, 2, 2);
    game.battlefield.add(firebender_id);

    // Add Firebending trigger (adds 2 red mana when attacking)
    {
        let creature = game.cards.get_mut(firebender_id).unwrap();
        creature.triggers.push(Trigger::new(
            TriggerEvent::Attacks,
            vec![Effect::Firebend {
                controller: PlayerId::new(0), // Placeholder
                amount: 2,
            }],
            "Firebending 2".to_string(),
        ));
    }

    // Create Lightning Bolt (R - instant, deal 3 damage)
    let bolt_id = game.next_card_id();
    let mut bolt = Card::new(bolt_id, "Lightning Bolt", alice);
    bolt.set_types(SmallVec::from_vec(vec![CardType::Instant]));
    bolt.mana_cost = ManaCost {
        red: 1,
        ..Default::default()
    };
    bolt.controller = alice;
    game.cards.insert(bolt_id, bolt);

    // Put bolt in hand
    if let Some(zones) = game.get_player_zones_mut(alice) {
        zones.hand.add(bolt_id);
    }

    // Verify initial state
    assert!(game.players[0].combat_mana_pool.is_none(), "No combat mana initially");
    assert_eq!(game.players[0].mana_pool.red, 0, "No regular mana initially");
    assert_eq!(game.players[1].life, 20, "Bob starts at 20 life");

    // Step 1: Attack triggers Firebending
    game.check_attack_triggers(firebender_id, alice)
        .expect("Attack triggers should succeed");

    // Verify combat mana was added
    assert_eq!(
        combat_red(&game.players[0]),
        2,
        "Should have 2 red combat mana from Firebending"
    );

    // Step 2: Cast Lightning Bolt using combat mana
    // First, set up target (Bob)
    let targets = vec![];

    // Move bolt from hand to stack and pay cost
    let cast_result = game.cast_spell(alice, bolt_id, targets);
    assert!(
        cast_result.is_ok(),
        "Should be able to cast Lightning Bolt with combat mana: {:?}",
        cast_result
    );

    // Verify combat mana was spent
    // We had 2 red combat mana, bolt costs 1 red, so 1 should remain
    assert_eq!(
        combat_red(&game.players[0]),
        1,
        "Should have 1 red combat mana remaining after casting bolt (2 - 1 = 1)"
    );

    // Verify bolt is on the stack
    assert!(game.stack.contains(bolt_id), "Lightning Bolt should be on the stack");

    // Resolve the bolt (simplified - just execute its effect)
    // In a real game, resolve_top_spell_from_stack would handle this
    let bolt_effect = Effect::DealDamage {
        target: mtg_engine::core::TargetRef::Player(bob),
        amount: 3,
    };
    game.execute_effect(&bolt_effect).expect("Bolt should deal damage");

    // Verify damage was dealt
    assert_eq!(
        game.players[1].life, 17,
        "Bob should be at 17 life after Lightning Bolt (20 - 3)"
    );
}

/// Test that combat mana can pay for a more expensive spell
#[test]
fn test_firebend_e2e_combat_mana_pays_expensive_spell() {
    use mtg_engine::core::{Card, CardType, Effect, ManaCost, Trigger, TriggerEvent};

    let mut game = GameState::new_two_player("Alice".to_string(), "Bob".to_string(), 20);
    let alice = game.players[0].id;

    // Create a 4/4 creature with Firebending X (X = power = 4)
    let firebender_id = create_creature(&mut game, "Firebending Master", alice, 4, 4);
    game.battlefield.add(firebender_id);

    // Add Firebending X trigger (amount=0 means use creature's power)
    {
        let creature = game.cards.get_mut(firebender_id).unwrap();
        creature.triggers.push(Trigger::new(
            TriggerEvent::Attacks,
            vec![Effect::Firebend {
                controller: PlayerId::new(0), // Placeholder
                amount: 0,                    // X = creature's power
            }],
            "Firebending X".to_string(),
        ));
    }

    // Create a 3R spell (4 total mana)
    let spell_id = game.next_card_id();
    let mut spell = Card::new(spell_id, "Fireblast", alice);
    spell.set_types(SmallVec::from_vec(vec![CardType::Instant]));
    spell.mana_cost = ManaCost {
        generic: 3,
        red: 1,
        ..Default::default()
    };
    spell.controller = alice;
    game.cards.insert(spell_id, spell);

    // Put spell in hand
    if let Some(zones) = game.get_player_zones_mut(alice) {
        zones.hand.add(spell_id);
    }

    // Attack triggers Firebending X (adds 4 red mana)
    game.check_attack_triggers(firebender_id, alice)
        .expect("Attack triggers should succeed");

    assert_eq!(
        combat_red(&game.players[0]),
        4,
        "Should have 4 red combat mana from Firebending X"
    );

    // Cast the 3R spell using combat mana
    let cast_result = game.cast_spell(alice, spell_id, vec![]);
    assert!(
        cast_result.is_ok(),
        "Should be able to cast 3R spell with 4 red combat mana: {:?}",
        cast_result
    );

    // Verify all combat mana was spent (1R for red, 3R for generic = 4R total)
    assert!(
        game.players[0].combat_mana_pool.is_none(),
        "All combat mana should be spent (pool deallocated)"
    );
}

/// Test that regular mana and combat mana work together
#[test]
fn test_firebend_e2e_combined_regular_and_combat_mana() {
    use mtg_engine::core::{Card, CardType, Effect, ManaCost, Trigger, TriggerEvent};

    let mut game = GameState::new_two_player("Alice".to_string(), "Bob".to_string(), 20);
    let alice = game.players[0].id;

    // Give Alice 2 regular red mana (from lands tapped earlier)
    game.players[0].mana_pool.red = 2;

    // Create a creature with Firebending 2
    let firebender_id = create_creature(&mut game, "Fire Elemental", alice, 3, 3);
    game.battlefield.add(firebender_id);

    {
        let creature = game.cards.get_mut(firebender_id).unwrap();
        creature.triggers.push(Trigger::new(
            TriggerEvent::Attacks,
            vec![Effect::Firebend {
                controller: PlayerId::new(0),
                amount: 2,
            }],
            "Firebending 2".to_string(),
        ));
    }

    // Create a 3R spell (needs 4 mana total)
    let spell_id = game.next_card_id();
    let mut spell = Card::new(spell_id, "Volcanic Hammer", alice);
    spell.set_types(SmallVec::from_vec(vec![CardType::Instant]));
    spell.mana_cost = ManaCost {
        generic: 3,
        red: 1,
        ..Default::default()
    };
    spell.controller = alice;
    game.cards.insert(spell_id, spell);

    if let Some(zones) = game.get_player_zones_mut(alice) {
        zones.hand.add(spell_id);
    }

    // Attack triggers Firebending
    game.check_attack_triggers(firebender_id, alice)
        .expect("Attack triggers should succeed");

    // Now have: 2 regular red + 2 combat red = 4 red total
    assert_eq!(game.players[0].mana_pool.red, 2, "2 regular red mana");
    assert_eq!(combat_red(&game.players[0]), 2, "2 combat red mana");

    // Cast 3R spell - should use regular mana first, then combat
    let cast_result = game.cast_spell(alice, spell_id, vec![]);
    assert!(cast_result.is_ok(), "Should cast with combined mana: {:?}", cast_result);

    // Should have spent: 1R (colored) + 3 generic (from remaining red)
    // Regular pool first: 2R used (1 for colored, 1 for generic) = 0 remaining
    // Combat pool: 2R used for generic = 0 remaining
    assert_eq!(
        game.players[0].mana_pool.red, 0,
        "Regular red mana should be spent first"
    );
    assert!(
        game.players[0].combat_mana_pool.is_none(),
        "Combat mana should be spent after regular"
    );
}
