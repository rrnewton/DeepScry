//! Test for Peter Porker (Spider-Ham) bug
//!
//! Bug report: Peter Porker incorrectly disappears when:
//! 1. It deals combat damage
//! 2. The Food token it created is sacrificed
//!
//! Expected: Peter Porker should remain on battlefield in both cases

use mtg_forge_rs::core::{CardName, PlayerId};
use mtg_forge_rs::game::game_init;
use mtg_forge_rs::game::state::GameState;

#[test]
fn test_peter_porker_persists_after_food_token_sacrifice() {
    // Setup: Create a game with Peter Porker on the battlefield
    let deck1_path = "decks/spider_ham_test.dck";
    let deck2_path = "decks/simple_bolt.dck";

    let mut game = game_init::init_game(deck1_path, deck2_path, Some(42))
        .expect("Failed to initialize game");

    let p1 = PlayerId::new(0);
    let p2 = PlayerId::new(1);

    // Give P1 enough mana to cast Peter Porker (1G)
    game.players[p1.as_usize()].mana_pool.green = 1;
    game.players[p1.as_usize()].mana_pool.colorless = 1;

    // Find Peter Porker in hand
    let p1_hand = game.get_zone_for_player(p1, mtg_forge_rs::core::Zone::Hand)
        .expect("Failed to get P1 hand");

    let peter_porker_id = p1_hand.cards.iter()
        .find(|&&card_id| {
            if let Ok(card) = game.cards.get(card_id) {
                card.name.to_string().contains("Peter Porker")
            } else {
                false
            }
        })
        .copied();

    if let Some(porker_id) = peter_porker_id {
        // Cast Peter Porker
        let result = game.cast_spell_simple(p1, porker_id);
        assert!(result.is_ok(), "Failed to cast Peter Porker: {:?}", result.err());

        // Verify Peter Porker is on battlefield
        assert!(game.battlefield.contains(porker_id),
            "Peter Porker should be on battlefield after casting");

        // Verify Food token was created (ETB trigger)
        let battlefield_count_before = game.battlefield.cards.len();
        println!("Battlefield cards after casting Peter Porker: {}", battlefield_count_before);

        // Should have Peter Porker + Food token = 2 cards
        // (assuming battlefield was empty before)
        let food_tokens: Vec<_> = game.battlefield.cards.iter()
            .filter(|&&card_id| {
                if let Ok(card) = game.cards.get(card_id) {
                    card.name.to_string().contains("Food")
                } else {
                    false
                }
            })
            .collect();

        println!("Found {} Food tokens", food_tokens.len());
        assert!(!food_tokens.is_empty(), "Food token should have been created");

        let food_token_id = *food_tokens[0];

        // Verify Peter Porker and Food token are different objects
        assert_ne!(porker_id, food_token_id,
            "Peter Porker and Food token should have different IDs");

        // Now sacrifice the Food token
        // Food token ability: {2}, {T}, Sacrifice this token: You gain 3 life

        // Give P1 mana to activate the Food token ability
        game.players[p1.as_usize()].mana_pool.colorless = 2;

        // Find the Food token's activated ability
        let food_card = game.cards.get(food_token_id).expect("Food token should exist");
        assert!(!food_card.activated_abilities.is_empty(),
            "Food token should have an activated ability");

        // Pay the cost manually (tap + sacrifice)
        // This simulates activating the ability
        let food_ability = &food_card.activated_abilities[0];
        let cost_result = game.pay_ability_cost(p1, food_token_id, &food_ability.cost);

        if let Err(e) = &cost_result {
            println!("Error paying Food token cost: {:?}", e);
        }
        assert!(cost_result.is_ok(), "Should be able to pay Food token cost");

        // Verify Food token was sacrificed (moved to graveyard)
        assert!(!game.battlefield.contains(food_token_id),
            "Food token should no longer be on battlefield after sacrifice");

        // **BUG CHECK**: Verify Peter Porker is STILL on battlefield
        assert!(game.battlefield.contains(porker_id),
            "BUG: Peter Porker should still be on battlefield after sacrificing Food token!");

        // Verify Peter Porker is the correct card (not corrupted)
        let porker_after = game.cards.get(porker_id).expect("Peter Porker should still exist");
        assert!(porker_after.name.to_string().contains("Peter Porker"),
            "Peter Porker card should still have correct name");
        assert_eq!(porker_after.power, 2, "Peter Porker should still be 2/2");
        assert_eq!(porker_after.toughness, 2, "Peter Porker should still be 2/2");

    } else {
        panic!("Peter Porker not found in P1's hand");
    }
}

#[test]
fn test_token_gets_unique_id() {
    // Simpler test: verify tokens get unique IDs different from the creature that creates them
    let deck1_path = "decks/spider_ham_test.dck";
    let deck2_path = "decks/simple_bolt.dck";

    let mut game = game_init::init_game(deck1_path, deck2_path, Some(42))
        .expect("Failed to initialize game");

    // Record all entity IDs currently in use
    let mut all_ids_before: Vec<u32> = game.cards.iter()
        .map(|(id, _)| id.as_u32())
        .collect();
    all_ids_before.sort();

    println!("Entity IDs before token creation: {:?}", all_ids_before);

    // Create a token using the token creation effect
    use mtg_forge_rs::core::Effect;
    let p1 = PlayerId::new(0);

    let create_token_effect = Effect::CreateToken {
        controller: p1,
        token_script: "c_a_food_sac".to_string(),
        amount: 1,
    };

    let result = game.execute_effect(&create_token_effect, p1, mtg_forge_rs::core::CardId::new(0));
    assert!(result.is_ok(), "Token creation should succeed: {:?}", result.err());

    // Get all IDs after token creation
    let mut all_ids_after: Vec<u32> = game.cards.iter()
        .map(|(id, _)| id.as_u32())
        .collect();
    all_ids_after.sort();

    println!("Entity IDs after token creation: {:?}", all_ids_after);

    // Find the new ID (token)
    let new_ids: Vec<u32> = all_ids_after.iter()
        .filter(|id| !all_ids_before.contains(id))
        .copied()
        .collect();

    assert_eq!(new_ids.len(), 1, "Exactly one new entity should be created (the token)");
    println!("Token got ID: {}", new_ids[0]);

    // Verify the token is unique and not reusing any existing ID
    assert!(!all_ids_before.contains(&new_ids[0]),
        "Token ID should be unique and not reuse existing IDs");
}
