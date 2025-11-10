//! End-to-end test for Spider-Suit Equipment functionality
//!
//! This test verifies that Equipment artifacts:
//! 1. Can be cast from hand
//! 2. Properly enter the battlefield when resolved
//! 3. (TODO) Can be attached to creatures via Equip ability
//! 4. (TODO) Grant stat bonuses to equipped creatures
//!
//! ## Current Status
//!
//! ✅ **Working**: Equipment casting and resolution
//! ❌ **Not Implemented**: Equipment attachment system
//!
//! ## What's Missing
//!
//! The game engine currently lacks:
//! - Card.attached_to field to track Equipment→Creature relationships
//! - Equip activated ability implementation
//! - Continuous effects from Equipment to equipped creature
//! - State-based action to detach Equipment when creature leaves battlefield
//!
//! See tracking issue mtg-TODO for Equipment implementation.

use mtg_forge_rs::core::{Card, CardName, CardType, ManaCost, Subtype};
use mtg_forge_rs::game::GameState;
use smallvec::SmallVec;

#[test]
fn test_spider_suit_enters_battlefield() {
    // Setup: Create game with Spider-Suit in hand and mana available
    let mut game = GameState::new_two_player("Player 1".to_string(), "Player 2".to_string(), 20);
    let p1_id = game.players[0].id;

    // Create Spider-Suit (costs {1}, Equipment subtype)
    let spider_suit_id = game.cards.next_id();
    let mut spider_suit = Card::new(spider_suit_id, CardName::from("Spider-Suit"), p1_id);
    spider_suit.mana_cost = ManaCost::from_string("1");
    spider_suit.types = SmallVec::from_vec(vec![CardType::Artifact]);
    spider_suit.subtypes = SmallVec::from_vec(vec![Subtype::from("Equipment")]);
    spider_suit.controller = p1_id;
    game.cards.insert(spider_suit_id, spider_suit);

    // Put Spider-Suit in hand
    if let Some(zones) = game.get_player_zones_mut(p1_id) {
        zones.hand.add(spider_suit_id);
    }

    // Add mana to pool
    if let Ok(player) = game.get_player_mut(p1_id) {
        player.mana_pool.red = 1;
    }

    // Verify initial state
    assert!(
        game.get_player_zones(p1_id).unwrap().hand.contains(spider_suit_id),
        "Spider-Suit should start in hand"
    );
    assert!(
        !game.battlefield.contains(spider_suit_id),
        "Spider-Suit should not be on battlefield before casting"
    );

    // Action: Cast Spider-Suit
    let cast_result = game.cast_spell(p1_id, spider_suit_id, Vec::new());
    assert!(
        cast_result.is_ok(),
        "Should successfully cast Spider-Suit: {:?}",
        cast_result
    );

    // Verify on stack
    assert!(
        game.stack.contains(spider_suit_id),
        "Spider-Suit should be on stack after casting"
    );
    assert!(
        !game.get_player_zones(p1_id).unwrap().hand.contains(spider_suit_id),
        "Spider-Suit should no longer be in hand"
    );

    // Action: Resolve Spider-Suit
    let resolve_result = game.resolve_spell(spider_suit_id, &[]);
    assert!(
        resolve_result.is_ok(),
        "Should successfully resolve Spider-Suit: {:?}",
        resolve_result
    );

    // Verify final state
    assert!(
        !game.stack.contains(spider_suit_id),
        "Spider-Suit should no longer be on stack"
    );
    assert!(
        game.battlefield.contains(spider_suit_id),
        "Spider-Suit should be on battlefield after resolution"
    );

    // Verify card properties on battlefield
    let card = game.cards.get(spider_suit_id).expect("Card should exist");
    assert!(card.is_type(&CardType::Artifact), "Spider-Suit should be an Artifact");
    assert!(
        card.subtypes.contains(&Subtype::from("Equipment")),
        "Spider-Suit should have Equipment subtype"
    );
}

#[test]
fn test_spider_suit_full_cast_resolve_workflow() {
    // This test verifies the complete workflow of casting and resolving Equipment
    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1_id = game.players[0].id;

    // Create Spider-Suit
    let spider_suit_id = game.cards.next_id();
    let mut spider_suit = Card::new(spider_suit_id, CardName::from("Spider-Suit"), p1_id);
    spider_suit.mana_cost = ManaCost::from_string("1");
    spider_suit.types = SmallVec::from_vec(vec![CardType::Artifact]);
    spider_suit.subtypes = SmallVec::from_vec(vec![Subtype::from("Equipment")]);
    spider_suit.controller = p1_id;
    game.cards.insert(spider_suit_id, spider_suit);

    // Put in hand and add mana
    if let Some(zones) = game.get_player_zones_mut(p1_id) {
        zones.hand.add(spider_suit_id);
    }
    if let Ok(player) = game.get_player_mut(p1_id) {
        player.mana_pool.colorless = 1;
    }

    // Cast and resolve in one workflow
    game.cast_spell(p1_id, spider_suit_id, Vec::new())
        .expect("Cast should succeed");
    game.resolve_spell(spider_suit_id, &[]).expect("Resolve should succeed");

    // Final verification
    assert!(
        game.battlefield.contains(spider_suit_id),
        "Equipment should be on battlefield"
    );

    // Verify it's not in any other zone
    let zones = game.get_player_zones(p1_id).unwrap();
    assert!(!zones.hand.contains(spider_suit_id), "Should not be in hand");
    assert!(!zones.graveyard.contains(spider_suit_id), "Should not be in graveyard");
    assert!(!zones.exile.contains(spider_suit_id), "Should not be in exile");
    assert!(!game.stack.contains(spider_suit_id), "Should not be on stack");
}

// TODO(mtg-TODO): Add test for Equipment attachment when implemented
// #[test]
// #[ignore = "Equipment attachment not yet implemented"]
// fn test_spider_suit_equip_and_buff() {
//     // Setup: Spider-Suit on battlefield, Spider-Punk on battlefield, mana for equip
//     // Action: Activate equip ability targeting Spider-Punk
//     // Verify: Spider-Punk gains +2/+2 and Spider Hero types
//     // Verify: Spider-Punk's combat damage reflects the buff
// }
