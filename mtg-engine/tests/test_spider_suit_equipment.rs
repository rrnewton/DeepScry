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

/// Test Equipment attachment basics
#[test]
fn test_equipment_attachment() {
    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1_id = game.players[0].id;

    // Create Spider-Suit (Equipment)
    let spider_suit_id = game.cards.next_id();
    let mut spider_suit = Card::new(spider_suit_id, CardName::from("Spider-Suit"), p1_id);
    spider_suit.types = SmallVec::from_vec(vec![CardType::Artifact]);
    spider_suit.subtypes = SmallVec::from_vec(vec![Subtype::from("Equipment")]);
    spider_suit.controller = p1_id;
    game.cards.insert(spider_suit_id, spider_suit);

    // Create Spider-Punk (2/1 Creature)
    let spider_punk_id = game.cards.next_id();
    let mut spider_punk = Card::new(spider_punk_id, CardName::from("Spider-Punk"), p1_id);
    spider_punk.types = SmallVec::from_vec(vec![CardType::Creature]);
    spider_punk.power = Some(2);
    spider_punk.toughness = Some(1);
    spider_punk.controller = p1_id;
    game.cards.insert(spider_punk_id, spider_punk);

    // Put both on battlefield
    game.battlefield.add(spider_suit_id);
    game.battlefield.add(spider_punk_id);

    // Verify initial state
    assert!(
        game.battlefield.contains(spider_suit_id),
        "Spider-Suit should be on battlefield"
    );
    assert!(
        game.battlefield.contains(spider_punk_id),
        "Spider-Punk should be on battlefield"
    );

    let equipment = game.cards.get(spider_suit_id).expect("Equipment should exist");
    assert!(!equipment.is_attached(), "Equipment should not be attached initially");

    // Attach Equipment to creature
    let attach_result = game.attach_equipment(spider_suit_id, spider_punk_id);
    assert!(
        attach_result.is_ok(),
        "Should successfully attach Equipment: {:?}",
        attach_result
    );

    // Verify attachment
    let equipment = game.cards.get(spider_suit_id).expect("Equipment should exist");
    assert!(equipment.is_attached(), "Equipment should now be attached");
    assert_eq!(
        equipment.get_attached_to(),
        Some(spider_punk_id),
        "Equipment should be attached to Spider-Punk"
    );

    // Verify Equipment is found by get_attached_equipment
    let attached = game.get_attached_equipment(spider_punk_id);
    assert_eq!(attached.len(), 1, "Should have one Equipment attached");
    assert_eq!(attached[0], spider_suit_id, "Should be Spider-Suit");

    // Test detachment
    let detach_result = game.detach_equipment(spider_suit_id);
    assert!(
        detach_result.is_ok(),
        "Should successfully detach Equipment: {:?}",
        detach_result
    );

    // Verify detachment
    let equipment = game.cards.get(spider_suit_id).expect("Equipment should exist");
    assert!(!equipment.is_attached(), "Equipment should not be attached after detach");
    assert_eq!(
        equipment.get_attached_to(),
        None,
        "Equipment should not be attached to anything"
    );

    // Equipment should still be on battlefield
    assert!(
        game.battlefield.contains(spider_suit_id),
        "Equipment should remain on battlefield after detach"
    );
}

/// Test multiple Equipment on same creature
#[test]
fn test_multiple_equipment() {
    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1_id = game.players[0].id;

    // Create two Equipment
    let equip1_id = game.cards.next_id();
    let mut equip1 = Card::new(equip1_id, CardName::from("Sword"), p1_id);
    equip1.types = SmallVec::from_vec(vec![CardType::Artifact]);
    equip1.subtypes = SmallVec::from_vec(vec![Subtype::from("Equipment")]);
    equip1.controller = p1_id;
    game.cards.insert(equip1_id, equip1);

    let equip2_id = game.cards.next_id();
    let mut equip2 = Card::new(equip2_id, CardName::from("Shield"), p1_id);
    equip2.types = SmallVec::from_vec(vec![CardType::Artifact]);
    equip2.subtypes = SmallVec::from_vec(vec![Subtype::from("Equipment")]);
    equip2.controller = p1_id;
    game.cards.insert(equip2_id, equip2);

    // Create creature
    let creature_id = game.cards.next_id();
    let mut creature = Card::new(creature_id, CardName::from("Bear"), p1_id);
    creature.types = SmallVec::from_vec(vec![CardType::Creature]);
    creature.power = Some(2);
    creature.toughness = Some(2);
    creature.controller = p1_id;
    game.cards.insert(creature_id, creature);

    // Put all on battlefield
    game.battlefield.add(equip1_id);
    game.battlefield.add(equip2_id);
    game.battlefield.add(creature_id);

    // Attach both Equipment
    game.attach_equipment(equip1_id, creature_id)
        .expect("Should attach first Equipment");
    game.attach_equipment(equip2_id, creature_id)
        .expect("Should attach second Equipment");

    // Verify both are attached
    let attached = game.get_attached_equipment(creature_id);
    assert_eq!(attached.len(), 2, "Should have two Equipment attached");
    assert!(
        attached.contains(&equip1_id),
        "Should include first Equipment"
    );
    assert!(
        attached.contains(&equip2_id),
        "Should include second Equipment"
    );
}

/// Test Equipment buffs
#[test]
fn test_spider_suit_buff() {
    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1_id = game.players[0].id;

    // Create Spider-Suit (Equipment with +2/+2)
    let spider_suit_id = game.cards.next_id();
    let mut spider_suit = Card::new(spider_suit_id, CardName::from("Spider-Suit"), p1_id);
    spider_suit.types = SmallVec::from_vec(vec![CardType::Artifact]);
    spider_suit.subtypes = SmallVec::from_vec(vec![Subtype::from("Equipment")]);
    spider_suit.controller = p1_id;
    game.cards.insert(spider_suit_id, spider_suit);

    // Create Spider-Punk (2/1 Creature)
    let spider_punk_id = game.cards.next_id();
    let mut spider_punk = Card::new(spider_punk_id, CardName::from("Spider-Punk"), p1_id);
    spider_punk.types = SmallVec::from_vec(vec![CardType::Creature]);
    spider_punk.power = Some(2);
    spider_punk.toughness = Some(1);
    spider_punk.controller = p1_id;
    game.cards.insert(spider_punk_id, spider_punk);

    // Put both on battlefield
    game.battlefield.add(spider_suit_id);
    game.battlefield.add(spider_punk_id);

    // Check base stats
    let creature = game.cards.get(spider_punk_id).expect("Creature should exist");
    assert_eq!(creature.current_power(), 2, "Base power should be 2");
    assert_eq!(creature.current_toughness(), 1, "Base toughness should be 1");

    // Check effective stats without Equipment
    assert_eq!(
        game.get_effective_power(spider_punk_id).unwrap(),
        2,
        "Effective power without Equipment should be 2"
    );
    assert_eq!(
        game.get_effective_toughness(spider_punk_id).unwrap(),
        1,
        "Effective toughness without Equipment should be 1"
    );

    // Attach Equipment
    game.attach_equipment(spider_suit_id, spider_punk_id)
        .expect("Should attach Equipment");

    // Check effective stats WITH Equipment (+2/+2 from Spider-Suit)
    assert_eq!(
        game.get_effective_power(spider_punk_id).unwrap(),
        4,
        "Effective power with Spider-Suit should be 4 (2 + 2)"
    );
    assert_eq!(
        game.get_effective_toughness(spider_punk_id).unwrap(),
        3,
        "Effective toughness with Spider-Suit should be 3 (1 + 2)"
    );

    // Detach Equipment
    game.detach_equipment(spider_suit_id)
        .expect("Should detach Equipment");

    // Check stats return to normal
    assert_eq!(
        game.get_effective_power(spider_punk_id).unwrap(),
        2,
        "Effective power after detachment should be 2"
    );
    assert_eq!(
        game.get_effective_toughness(spider_punk_id).unwrap(),
        1,
        "Effective toughness after detachment should be 1"
    );
}

/// Test multiple Equipment buffs stack
#[test]
fn test_multiple_equipment_buffs() {
    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1_id = game.players[0].id;

    // Create two Spider-Suits
    let suit1_id = game.cards.next_id();
    let mut suit1 = Card::new(suit1_id, CardName::from("Spider-Suit"), p1_id);
    suit1.types = SmallVec::from_vec(vec![CardType::Artifact]);
    suit1.subtypes = SmallVec::from_vec(vec![Subtype::from("Equipment")]);
    suit1.controller = p1_id;
    game.cards.insert(suit1_id, suit1);

    let suit2_id = game.cards.next_id();
    let mut suit2 = Card::new(suit2_id, CardName::from("Spider-Suit"), p1_id);
    suit2.types = SmallVec::from_vec(vec![CardType::Artifact]);
    suit2.subtypes = SmallVec::from_vec(vec![Subtype::from("Equipment")]);
    suit2.controller = p1_id;
    game.cards.insert(suit2_id, suit2);

    // Create Bear (2/2 Creature)
    let bear_id = game.cards.next_id();
    let mut bear = Card::new(bear_id, CardName::from("Bear"), p1_id);
    bear.types = SmallVec::from_vec(vec![CardType::Creature]);
    bear.power = Some(2);
    bear.toughness = Some(2);
    bear.controller = p1_id;
    game.cards.insert(bear_id, bear);

    // Put all on battlefield
    game.battlefield.add(suit1_id);
    game.battlefield.add(suit2_id);
    game.battlefield.add(bear_id);

    // Attach both Equipment
    game.attach_equipment(suit1_id, bear_id)
        .expect("Should attach first Equipment");
    game.attach_equipment(suit2_id, bear_id)
        .expect("Should attach second Equipment");

    // Check stats with both Equipment (+2/+2 + +2/+2 = +4/+4)
    assert_eq!(
        game.get_effective_power(bear_id).unwrap(),
        6,
        "Effective power with 2 Spider-Suits should be 6 (2 + 2 + 2)"
    );
    assert_eq!(
        game.get_effective_toughness(bear_id).unwrap(),
        6,
        "Effective toughness with 2 Spider-Suits should be 6 (2 + 2 + 2)"
    );
}
