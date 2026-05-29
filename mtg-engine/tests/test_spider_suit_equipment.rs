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

use mtg_engine::core::{
    AffectedSelector, Card, CardId, CardName, CardType, ManaCost, PlayerId, StaticAbility, Subtype,
};
use mtg_engine::game::GameState;
use mtg_engine::zones::Zone;
use smallvec::SmallVec;

/// Helper function to create a Spider-Suit Equipment card with its static ability
fn create_spider_suit(id: CardId, owner: PlayerId) -> Card {
    let mut spider_suit = Card::new(id, CardName::from("Spider-Suit"), owner);
    spider_suit.mana_cost = ManaCost::from_string("1");
    spider_suit.set_types(SmallVec::from_vec(vec![CardType::Artifact]));
    spider_suit.set_subtypes(SmallVec::from_vec(vec![Subtype::from("Equipment")]));

    // Add static ability: +2/+2 to equipped creature
    // Corresponds to: S:Mode$ Continuous | Affected$ Creature.EquippedBy | AddPower$ 2 | AddToughness$ 2
    spider_suit.static_abilities.push(StaticAbility::ModifyPT {
        affected: AffectedSelector::CreatureEquippedBy,
        power: 2,
        toughness: 2,
        description: "Equipped creature gets +2/+2".to_string(),
        condition: None,
    });

    spider_suit
}

#[test]
fn test_spider_suit_enters_battlefield() {
    // Setup: Create game with Spider-Suit in hand and mana available
    let mut game = GameState::new_two_player("Player 1".to_string(), "Player 2".to_string(), 20);
    let p1_id = game.players[0].id;

    // Create Spider-Suit (costs {1}, Equipment subtype)
    let spider_suit_id = game.cards.next_id();
    let mut spider_suit = Card::new(spider_suit_id, CardName::from("Spider-Suit"), p1_id);
    spider_suit.mana_cost = ManaCost::from_string("1");
    spider_suit.set_types(SmallVec::from_vec(vec![CardType::Artifact]));
    spider_suit.set_subtypes(SmallVec::from_vec(vec![Subtype::from("Equipment")]));
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
    spider_suit.set_types(SmallVec::from_vec(vec![CardType::Artifact]));
    spider_suit.set_subtypes(SmallVec::from_vec(vec![Subtype::from("Equipment")]));
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
    spider_suit.set_types(SmallVec::from_vec(vec![CardType::Artifact]));
    spider_suit.set_subtypes(SmallVec::from_vec(vec![Subtype::from("Equipment")]));
    spider_suit.controller = p1_id;
    game.cards.insert(spider_suit_id, spider_suit);

    // Create Spider-Punk (2/1 Creature)
    let spider_punk_id = game.cards.next_id();
    let mut spider_punk = Card::new(spider_punk_id, CardName::from("Spider-Punk"), p1_id);
    spider_punk.set_types(SmallVec::from_vec(vec![CardType::Creature]));
    spider_punk.set_base_power(Some(2));
    spider_punk.set_base_toughness(Some(1));
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
    assert!(
        !equipment.is_attached(),
        "Equipment should not be attached after detach"
    );
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
    equip1.set_types(SmallVec::from_vec(vec![CardType::Artifact]));
    equip1.set_subtypes(SmallVec::from_vec(vec![Subtype::from("Equipment")]));
    equip1.controller = p1_id;
    game.cards.insert(equip1_id, equip1);

    let equip2_id = game.cards.next_id();
    let mut equip2 = Card::new(equip2_id, CardName::from("Shield"), p1_id);
    equip2.set_types(SmallVec::from_vec(vec![CardType::Artifact]));
    equip2.set_subtypes(SmallVec::from_vec(vec![Subtype::from("Equipment")]));
    equip2.controller = p1_id;
    game.cards.insert(equip2_id, equip2);

    // Create creature
    let creature_id = game.cards.next_id();
    let mut creature = Card::new(creature_id, CardName::from("Bear"), p1_id);
    creature.set_types(SmallVec::from_vec(vec![CardType::Creature]));
    creature.set_base_power(Some(2));
    creature.set_base_toughness(Some(2));
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
    assert!(attached.contains(&equip1_id), "Should include first Equipment");
    assert!(attached.contains(&equip2_id), "Should include second Equipment");
}

/// Test Equipment buffs
#[test]
fn test_spider_suit_buff() {
    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1_id = game.players[0].id;

    // Create Spider-Suit (Equipment with +2/+2)
    let spider_suit_id = game.cards.next_id();
    let mut spider_suit = create_spider_suit(spider_suit_id, p1_id);
    spider_suit.controller = p1_id;
    game.cards.insert(spider_suit_id, spider_suit);

    // Create Spider-Punk (2/1 Creature)
    let spider_punk_id = game.cards.next_id();
    let mut spider_punk = Card::new(spider_punk_id, CardName::from("Spider-Punk"), p1_id);
    spider_punk.set_types(SmallVec::from_vec(vec![CardType::Creature]));
    spider_punk.set_base_power(Some(2));
    spider_punk.set_base_toughness(Some(1));
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
    game.detach_equipment(spider_suit_id).expect("Should detach Equipment");

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
    let mut suit1 = create_spider_suit(suit1_id, p1_id);
    suit1.controller = p1_id;
    game.cards.insert(suit1_id, suit1);

    let suit2_id = game.cards.next_id();
    let mut suit2 = create_spider_suit(suit2_id, p1_id);
    suit2.controller = p1_id;
    game.cards.insert(suit2_id, suit2);

    // Create Bear (2/2 Creature)
    let bear_id = game.cards.next_id();
    let mut bear = Card::new(bear_id, CardName::from("Bear"), p1_id);
    bear.set_types(SmallVec::from_vec(vec![CardType::Creature]));
    bear.set_base_power(Some(2));
    bear.set_base_toughness(Some(2));
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

/// Test Equipment buffs are calculated correctly for combat
/// NOTE: This test verifies the buff calculation works, but doesn't test full combat workflow
/// (which requires player controllers). Combat integration is tested via e2e tests.
#[test]
fn test_equipment_combat_damage_calculation() {
    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1_id = game.players[0].id;

    // Create Spider-Suit (Equipment with +2/+2)
    let spider_suit_id = game.cards.next_id();
    let mut spider_suit = create_spider_suit(spider_suit_id, p1_id);
    spider_suit.controller = p1_id;
    game.cards.insert(spider_suit_id, spider_suit);

    // Create Spider-Punk (2/1 Creature)
    let spider_punk_id = game.cards.next_id();
    let mut spider_punk = Card::new(spider_punk_id, CardName::from("Spider-Punk"), p1_id);
    spider_punk.set_types(SmallVec::from_vec(vec![CardType::Creature]));
    spider_punk.set_base_power(Some(2));
    spider_punk.set_base_toughness(Some(1));
    spider_punk.controller = p1_id;
    game.cards.insert(spider_punk_id, spider_punk);

    // Put both on battlefield
    game.battlefield.add(spider_suit_id);
    game.battlefield.add(spider_punk_id);

    // Attach Equipment to Spider-Punk
    game.attach_equipment(spider_suit_id, spider_punk_id)
        .expect("Should attach Equipment");

    // Verify buffed power is calculated correctly for combat
    // Combat damage calculation in assign_combat_damage() uses get_effective_power()
    assert_eq!(
        game.get_effective_power(spider_punk_id).unwrap(),
        4,
        "Spider-Punk should have 4 power with Equipment (2 base + 2 from Equipment)"
    );

    // Verify base power unchanged
    let creature = game.cards.get(spider_punk_id).unwrap();
    assert_eq!(creature.current_power(), 2, "Base power should still be 2");
}

/// Test Equipment detaches when creature dies (state-based action)
#[test]
fn test_equipment_detaches_when_creature_dies() {
    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1_id = game.players[0].id;

    // Create Spider-Suit (Equipment)
    let spider_suit_id = game.cards.next_id();
    let mut spider_suit = Card::new(spider_suit_id, CardName::from("Spider-Suit"), p1_id);
    spider_suit.set_types(SmallVec::from_vec(vec![CardType::Artifact]));
    spider_suit.set_subtypes(SmallVec::from_vec(vec![Subtype::from("Equipment")]));
    spider_suit.controller = p1_id;
    game.cards.insert(spider_suit_id, spider_suit);

    // Create Spider-Punk (2/1 Creature)
    let spider_punk_id = game.cards.next_id();
    let mut spider_punk = Card::new(spider_punk_id, CardName::from("Spider-Punk"), p1_id);
    spider_punk.set_types(SmallVec::from_vec(vec![CardType::Creature]));
    spider_punk.set_base_power(Some(2));
    spider_punk.set_base_toughness(Some(1));
    spider_punk.controller = p1_id;
    game.cards.insert(spider_punk_id, spider_punk);

    // Put both on battlefield
    game.battlefield.add(spider_suit_id);
    game.battlefield.add(spider_punk_id);

    // Attach Equipment
    game.attach_equipment(spider_suit_id, spider_punk_id)
        .expect("Should attach Equipment");

    // Verify Equipment is attached
    let equipment = game.cards.get(spider_suit_id).expect("Equipment should exist");
    assert!(equipment.is_attached(), "Equipment should be attached");
    assert_eq!(equipment.get_attached_to(), Some(spider_punk_id));

    // Move creature to graveyard (simulating death)
    game.move_card(spider_punk_id, Zone::Battlefield, Zone::Graveyard, p1_id)
        .expect("Should move creature to graveyard");

    // Verify Equipment detached automatically (state-based action)
    let equipment = game.cards.get(spider_suit_id).expect("Equipment should exist");
    assert!(
        !equipment.is_attached(),
        "Equipment should auto-detach when creature dies"
    );
    assert_eq!(
        equipment.get_attached_to(),
        None,
        "Equipment should not be attached to anything"
    );

    // Equipment should still be on battlefield
    assert!(
        game.battlefield.contains(spider_suit_id),
        "Equipment should remain on battlefield"
    );

    // Creature should be in graveyard
    assert!(
        !game.battlefield.contains(spider_punk_id),
        "Creature should not be on battlefield"
    );
    let zones = game.get_player_zones(p1_id).unwrap();
    assert!(
        zones.graveyard.contains(spider_punk_id),
        "Creature should be in graveyard"
    );
}

// =========================================================================
// STATE-BASED SELECTOR TESTS
// Tests for Card.Self+equipped, Card.Self+enchanted, Creature.YouCtrl+equipped
// Related to mtg-147: Affected$ selector parsing
// =========================================================================

/// Helper to create a creature with SelfWhenEquipped static ability
/// Simulates cards like Leonin Lightbringer ("As long as ~ is equipped, it gets +1/+1")
fn create_self_when_equipped_creature(id: CardId, owner: PlayerId) -> Card {
    let mut creature = Card::new(id, CardName::from("Leonin Lightbringer"), owner);
    creature.set_types(SmallVec::from_vec(vec![CardType::Creature]));
    creature.set_base_power(Some(2));
    creature.set_base_toughness(Some(2));
    creature.controller = owner;

    // Add static ability: +1/+1 when equipped
    // Corresponds to: S:Mode$ Continuous | Affected$ Card.Self+equipped | AddPower$ 1 | AddToughness$ 1
    creature.static_abilities.push(StaticAbility::ModifyPT {
        affected: AffectedSelector::SelfWhenEquipped,
        power: 1,
        toughness: 1,
        description: "As long as ~ is equipped, it gets +1/+1".to_string(),
        condition: None,
    });

    creature
}

/// Test Card.Self+equipped selector - creature gets buff when equipped
#[test]
fn test_self_when_equipped_selector() {
    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1_id = game.players[0].id;

    // Create Leonin Lightbringer (2/2, gets +1/+1 when equipped)
    let creature_id = game.cards.next_id();
    let creature = create_self_when_equipped_creature(creature_id, p1_id);
    game.cards.insert(creature_id, creature);

    // Create simple Equipment (no static ability for simplicity)
    let equip_id = game.cards.next_id();
    let mut equipment = Card::new(equip_id, CardName::from("Simple Sword"), p1_id);
    equipment.set_types(SmallVec::from_vec(vec![CardType::Artifact]));
    equipment.set_subtypes(SmallVec::from_vec(vec![Subtype::from("Equipment")]));
    equipment.controller = p1_id;
    game.cards.insert(equip_id, equipment);

    // Put both on battlefield
    game.battlefield.add(creature_id);
    game.battlefield.add(equip_id);

    // Check stats WITHOUT equipment (should be base 2/2)
    assert_eq!(
        game.get_effective_power(creature_id).unwrap(),
        2,
        "Power without equipment should be 2"
    );
    assert_eq!(
        game.get_effective_toughness(creature_id).unwrap(),
        2,
        "Toughness without equipment should be 2"
    );

    // Attach Equipment
    game.attach_equipment(equip_id, creature_id)
        .expect("Should attach Equipment");

    // Check stats WITH equipment (should be 3/3: base 2/2 + 1/1 from SelfWhenEquipped)
    assert_eq!(
        game.get_effective_power(creature_id).unwrap(),
        3,
        "Power with equipment should be 3 (2 base + 1 from self-buff)"
    );
    assert_eq!(
        game.get_effective_toughness(creature_id).unwrap(),
        3,
        "Toughness with equipment should be 3 (2 base + 1 from self-buff)"
    );

    // Detach Equipment
    game.detach_equipment(equip_id).expect("Should detach Equipment");

    // Check stats return to base
    assert_eq!(
        game.get_effective_power(creature_id).unwrap(),
        2,
        "Power after detachment should return to 2"
    );
    assert_eq!(
        game.get_effective_toughness(creature_id).unwrap(),
        2,
        "Toughness after detachment should return to 2"
    );
}

/// Test Card.Self+equipped stacks with Equipment buff
/// When a creature has "gets +1/+1 when equipped" AND the equipment gives +2/+2
#[test]
fn test_self_when_equipped_stacks_with_equipment_buff() {
    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1_id = game.players[0].id;

    // Create Leonin Lightbringer (2/2, gets +1/+1 when equipped)
    let creature_id = game.cards.next_id();
    let creature = create_self_when_equipped_creature(creature_id, p1_id);
    game.cards.insert(creature_id, creature);

    // Create Spider-Suit (Equipment with +2/+2)
    let spider_suit_id = game.cards.next_id();
    let mut spider_suit = create_spider_suit(spider_suit_id, p1_id);
    spider_suit.controller = p1_id;
    game.cards.insert(spider_suit_id, spider_suit);

    // Put both on battlefield
    game.battlefield.add(creature_id);
    game.battlefield.add(spider_suit_id);

    // Check base stats
    assert_eq!(
        game.get_effective_power(creature_id).unwrap(),
        2,
        "Power without equipment should be 2"
    );

    // Attach Spider-Suit
    game.attach_equipment(spider_suit_id, creature_id)
        .expect("Should attach Spider-Suit");

    // Check stacked stats:
    // Base: 2/2
    // From SelfWhenEquipped: +1/+1
    // From Spider-Suit: +2/+2
    // Total: 5/5
    assert_eq!(
        game.get_effective_power(creature_id).unwrap(),
        5,
        "Power should be 5 (2 base + 1 self-buff + 2 equipment)"
    );
    assert_eq!(
        game.get_effective_toughness(creature_id).unwrap(),
        5,
        "Toughness should be 5 (2 base + 1 self-buff + 2 equipment)"
    );
}

/// Helper to create an enchantment (Aura) that can be attached
#[allow(dead_code)]
fn create_test_aura(id: CardId, owner: PlayerId, power_buff: i32, toughness_buff: i32) -> Card {
    let mut aura = Card::new(id, CardName::from("Test Aura"), owner);
    aura.set_types(SmallVec::from_vec(vec![CardType::Enchantment]));
    aura.set_subtypes(SmallVec::from_vec(vec![Subtype::from("Aura")]));
    aura.controller = owner;

    // Add static ability: +P/+T to enchanted creature
    aura.static_abilities.push(StaticAbility::ModifyPT {
        affected: AffectedSelector::CreatureEnchantedBy,
        power: power_buff,
        toughness: toughness_buff,
        description: format!("Enchanted creature gets +{}/+{}", power_buff, toughness_buff),
        condition: None,
    });

    aura
}

/// Helper to create a creature with SelfWhenEnchanted static ability
/// Simulates cards like Thran Golem ("As long as ~ is enchanted, it gets +2/+2")
fn create_self_when_enchanted_creature(id: CardId, owner: PlayerId) -> Card {
    let mut creature = Card::new(id, CardName::from("Thran Golem"), owner);
    creature.set_types(SmallVec::from_vec(vec![CardType::Creature]));
    creature.set_base_power(Some(3));
    creature.set_base_toughness(Some(3));
    creature.controller = owner;

    // Add static ability: +2/+2 when enchanted
    // Corresponds to: S:Mode$ Continuous | Affected$ Card.Self+enchanted | AddPower$ 2 | AddToughness$ 2
    creature.static_abilities.push(StaticAbility::ModifyPT {
        affected: AffectedSelector::SelfWhenEnchanted,
        power: 2,
        toughness: 2,
        description: "As long as ~ is enchanted, it gets +2/+2".to_string(),
        condition: None,
    });

    creature
}

/// Test Card.Self+enchanted selector - creature gets buff when enchanted
#[test]
fn test_self_when_enchanted_selector() {
    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1_id = game.players[0].id;

    // Create Thran Golem (3/3, gets +2/+2 when enchanted)
    let creature_id = game.cards.next_id();
    let creature = create_self_when_enchanted_creature(creature_id, p1_id);
    game.cards.insert(creature_id, creature);

    // Create simple Aura (no buff of its own)
    let aura_id = game.cards.next_id();
    let mut aura = Card::new(aura_id, CardName::from("Flight"), p1_id);
    aura.set_types(SmallVec::from_vec(vec![CardType::Enchantment]));
    aura.set_subtypes(SmallVec::from_vec(vec![Subtype::from("Aura")]));
    aura.controller = p1_id;
    game.cards.insert(aura_id, aura);

    // Put both on battlefield
    game.battlefield.add(creature_id);
    game.battlefield.add(aura_id);

    // Check stats WITHOUT aura (should be base 3/3)
    assert_eq!(
        game.get_effective_power(creature_id).unwrap(),
        3,
        "Power without aura should be 3"
    );
    assert_eq!(
        game.get_effective_toughness(creature_id).unwrap(),
        3,
        "Toughness without aura should be 3"
    );

    // Attach Aura (uses same attach function as Equipment)
    game.attach_equipment(aura_id, creature_id).expect("Should attach Aura");

    // Check stats WITH aura (should be 5/5: base 3/3 + 2/2 from SelfWhenEnchanted)
    assert_eq!(
        game.get_effective_power(creature_id).unwrap(),
        5,
        "Power with aura should be 5 (3 base + 2 from self-buff)"
    );
    assert_eq!(
        game.get_effective_toughness(creature_id).unwrap(),
        5,
        "Toughness with aura should be 5 (3 base + 2 from self-buff)"
    );

    // Detach Aura (uses same detach function as Equipment)
    game.detach_equipment(aura_id).expect("Should detach Aura");

    // Check stats return to base
    assert_eq!(
        game.get_effective_power(creature_id).unwrap(),
        3,
        "Power after detachment should return to 3"
    );
    assert_eq!(
        game.get_effective_toughness(creature_id).unwrap(),
        3,
        "Toughness after detachment should return to 3"
    );
}

/// Helper to create a creature with EquippedCreaturesYouControl buff
/// Simulates cards like Kemba, Kha Enduring ("Equipped creatures you control get +1/+1")
fn create_equipped_creatures_lord(id: CardId, owner: PlayerId) -> Card {
    let mut lord = Card::new(id, CardName::from("Kemba, Kha Enduring"), owner);
    lord.set_types(SmallVec::from_vec(vec![CardType::Creature]));
    lord.set_subtypes(SmallVec::from_vec(vec![Subtype::from("Cat"), Subtype::from("Cleric")]));
    lord.set_base_power(Some(2));
    lord.set_base_toughness(Some(4));
    lord.controller = owner;

    // Add static ability: Equipped creatures you control get +1/+1
    // Corresponds to: S:Mode$ Continuous | Affected$ Creature.YouCtrl+equipped | AddPower$ 1 | AddToughness$ 1
    lord.static_abilities.push(StaticAbility::ModifyPT {
        affected: AffectedSelector::EquippedCreaturesYouControl,
        power: 1,
        toughness: 1,
        description: "Equipped creatures you control get +1/+1".to_string(),
        condition: None,
    });

    lord
}

/// Test Creature.YouCtrl+equipped selector - only equipped creatures get buff
#[test]
fn test_equipped_creatures_you_control_selector() {
    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1_id = game.players[0].id;

    // Create Kemba, Kha Enduring (lord that buffs equipped creatures)
    let lord_id = game.cards.next_id();
    let lord = create_equipped_creatures_lord(lord_id, p1_id);
    game.cards.insert(lord_id, lord);

    // Create two creatures: one will be equipped, one won't
    let creature1_id = game.cards.next_id();
    let mut creature1 = Card::new(creature1_id, CardName::from("Bear"), p1_id);
    creature1.set_types(SmallVec::from_vec(vec![CardType::Creature]));
    creature1.set_base_power(Some(2));
    creature1.set_base_toughness(Some(2));
    creature1.controller = p1_id;
    game.cards.insert(creature1_id, creature1);

    let creature2_id = game.cards.next_id();
    let mut creature2 = Card::new(creature2_id, CardName::from("Wolf"), p1_id);
    creature2.set_types(SmallVec::from_vec(vec![CardType::Creature]));
    creature2.set_base_power(Some(2));
    creature2.set_base_toughness(Some(2));
    creature2.controller = p1_id;
    game.cards.insert(creature2_id, creature2);

    // Create Equipment
    let equip_id = game.cards.next_id();
    let mut equipment = Card::new(equip_id, CardName::from("Short Sword"), p1_id);
    equipment.set_types(SmallVec::from_vec(vec![CardType::Artifact]));
    equipment.set_subtypes(SmallVec::from_vec(vec![Subtype::from("Equipment")]));
    equipment.controller = p1_id;
    game.cards.insert(equip_id, equipment);

    // Put all on battlefield
    game.battlefield.add(lord_id);
    game.battlefield.add(creature1_id);
    game.battlefield.add(creature2_id);
    game.battlefield.add(equip_id);

    // Check stats before equipping (both creatures should be 2/2)
    assert_eq!(
        game.get_effective_power(creature1_id).unwrap(),
        2,
        "Bear power without equipment should be 2"
    );
    assert_eq!(
        game.get_effective_power(creature2_id).unwrap(),
        2,
        "Wolf power without equipment should be 2"
    );
    // Lord itself is not equipped, so shouldn't get the buff
    assert_eq!(
        game.get_effective_power(lord_id).unwrap(),
        2,
        "Lord power without equipment should be 2"
    );

    // Attach Equipment to Bear only
    game.attach_equipment(equip_id, creature1_id)
        .expect("Should attach Equipment to Bear");

    // Now Bear should get +1/+1 from the lord, Wolf should not
    assert_eq!(
        game.get_effective_power(creature1_id).unwrap(),
        3,
        "Bear power with equipment should be 3 (2 base + 1 from lord)"
    );
    assert_eq!(
        game.get_effective_toughness(creature1_id).unwrap(),
        3,
        "Bear toughness with equipment should be 3 (2 base + 1 from lord)"
    );
    assert_eq!(
        game.get_effective_power(creature2_id).unwrap(),
        2,
        "Wolf power should still be 2 (not equipped)"
    );

    // Move equipment from Bear to Wolf
    game.detach_equipment(equip_id).expect("Should detach");
    game.attach_equipment(equip_id, creature2_id)
        .expect("Should attach to Wolf");

    // Now Wolf should get the buff, Bear should not
    assert_eq!(
        game.get_effective_power(creature1_id).unwrap(),
        2,
        "Bear power after losing equipment should be 2"
    );
    assert_eq!(
        game.get_effective_power(creature2_id).unwrap(),
        3,
        "Wolf power with equipment should be 3 (2 base + 1 from lord)"
    );
}

/// Regression test: an Equipment's Equip ability must not offer the creature it is
/// already attached to as a valid target. Re-equipping the same creature is a strictly
/// wasteful no-op (detach + reattach burns mana for no game effect). Filtering it out
/// keeps random/AI controllers from blowing mana on it. See bug-equipment-detach-reattach.
#[test]
fn test_equip_excludes_already_attached_creature() {
    use mtg_engine::core::{ActivatedAbility, Cost, Effect, ManaCost};

    let mut game = GameState::new_two_player("P1".to_string(), "P2".to_string(), 20);
    let p1_id = game.players[0].id;

    // Create Equipment with an Equip {1} activated ability ("Trusty Boomerang"-like)
    let equip_id = game.cards.next_id();
    let mut equipment = Card::new(equip_id, CardName::from("Trusty Boomerang"), p1_id);
    equipment.set_types(SmallVec::from_vec(vec![CardType::Artifact]));
    equipment.set_subtypes(SmallVec::from_vec(vec![Subtype::from("Equipment")]));
    equipment.controller = p1_id;
    equipment.activated_abilities.push(ActivatedAbility::new_sorcery_speed(
        Cost::Mana(ManaCost::from_string("1")),
        vec![Effect::AttachEquipment {
            source_equipment: equip_id,
            target_creature: CardId::new(0), // Placeholder - filled in during activation
        }],
        "Equip 1".to_string(),
    ));
    game.cards.insert(equip_id, equipment);

    // Two creatures controlled by p1
    let creature_a_id = game.cards.next_id();
    let mut creature_a = Card::new(creature_a_id, CardName::from("Knowledge Seeker"), p1_id);
    creature_a.set_types(SmallVec::from_vec(vec![CardType::Creature]));
    creature_a.set_base_power(Some(2));
    creature_a.set_base_toughness(Some(1));
    creature_a.controller = p1_id;
    game.cards.insert(creature_a_id, creature_a);

    let creature_b_id = game.cards.next_id();
    let mut creature_b = Card::new(creature_b_id, CardName::from("Forecasting Fortune Teller"), p1_id);
    creature_b.set_types(SmallVec::from_vec(vec![CardType::Creature]));
    creature_b.set_base_power(Some(1));
    creature_b.set_base_toughness(Some(3));
    creature_b.controller = p1_id;
    game.cards.insert(creature_b_id, creature_b);

    game.battlefield.add(equip_id);
    game.battlefield.add(creature_a_id);
    game.battlefield.add(creature_b_id);

    // Before any attachment: both creatures are valid equip targets
    let targets_before = game
        .get_valid_targets_for_ability(equip_id, 0)
        .expect("get_valid_targets_for_ability should succeed");
    assert!(
        targets_before.contains(&creature_a_id),
        "Knowledge Seeker should be a valid equip target before attachment"
    );
    assert!(
        targets_before.contains(&creature_b_id),
        "Forecasting Fortune Teller should be a valid equip target before attachment"
    );

    // Attach to creature_a
    game.attach_equipment(equip_id, creature_a_id)
        .expect("Should attach Equipment to Knowledge Seeker");

    // After attachment: creature_a (already attached) must be EXCLUDED, creature_b stays valid
    let targets_after = game
        .get_valid_targets_for_ability(equip_id, 0)
        .expect("get_valid_targets_for_ability should succeed");
    assert!(
        !targets_after.contains(&creature_a_id),
        "Knowledge Seeker (already equipped) must NOT be a valid equip target — \
         re-equipping the same creature is a wasteful no-op (detach+reattach for no effect). \
         Got targets: {:?}",
        targets_after
    );
    assert!(
        targets_after.contains(&creature_b_id),
        "Forecasting Fortune Teller (different creature) must still be a valid equip target — \
         moving the equipment to a different creature is a meaningful action. Got targets: {:?}",
        targets_after
    );
}
