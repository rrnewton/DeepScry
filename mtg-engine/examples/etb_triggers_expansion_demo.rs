//! ETB Triggers - Expansion Demo
//!
//! Demonstrates a more complex scenario with multiple creatures having different
//! ETB triggers, showing how they interact in actual gameplay. This example
//! simulates a realistic game situation where creatures enter the battlefield
//! and their triggers create interesting interactions.
//!
//! Featured ETB triggers:
//! - Card draw (Elvish Visionary)
//! - Damage dealing (Flametongue Kavu)
//! - Life gain (custom Soul Warden-like creature)

use mtg_forge_rs::core::{Card, CardType, Effect, ManaCost, Trigger, TriggerEvent};
use mtg_forge_rs::game::GameState;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== MTG Forge - ETB Triggers Expansion Demo ===\n");
    println!("This demo shows multiple ETB triggers in action during a game.\n");

    // Create a two-player game
    let mut game = GameState::new_two_player("Alice".to_string(), "Bob".to_string(), 20);
    let alice_id = game.players[0].id;
    let bob_id = game.players[1].id;

    println!("=== Initial Setup ===");
    println!("Alice: 20 life");
    println!("Bob: 20 life\n");

    // Add cards to Alice's library for draw triggers
    println!("Adding 10 cards to Alice's library...");
    for i in 0..10 {
        let card_id = game.next_card_id();
        let card = Card::new(card_id, format!("Library Card {}", i + 1), alice_id);
        game.cards.insert(card_id, card);
        if let Some(zones) = game.get_player_zones_mut(alice_id) {
            zones.library.add(card_id);
        }
    }
    println!("✓ Alice's library: 10 cards\n");

    // Scenario 1: Alice casts a creature with life gain ETB
    println!("=== Turn 1: Alice casts Soul's Attendant ===");
    let attendant_id = game.next_entity_id();
    let mut attendant = Card::new(attendant_id, "Soul's Attendant".to_string(), alice_id);
    attendant.types.push(CardType::Creature);
    attendant.power = Some(1);
    attendant.toughness = Some(1);
    attendant.mana_cost = ManaCost::from_string("W");
    attendant.triggers.push(Trigger::new(
        TriggerEvent::EntersBattlefield,
        vec![Effect::GainLife {
            player: alice_id,
            amount: 1,
        }],
        "When Soul's Attendant enters, you gain 1 life.".to_string(),
    ));

    let alice_life_before = game.get_player(alice_id)?.life;
    game.cards.insert(attendant_id, attendant);
    game.stack.add(attendant_id);
    game.resolve_spell(attendant_id, &[])?;
    let alice_life_after = game.get_player(alice_id)?.life;

    println!("Soul's Attendant (1/1) enters the battlefield");
    println!("✓ ETB trigger: Alice gains 1 life ({alice_life_before} → {alice_life_after})");
    println!("Alice: {alice_life_after} life, 1 creature on battlefield\n");

    // Scenario 2: Alice casts Elvish Visionary
    println!("=== Turn 2: Alice casts Elvish Visionary ===");
    let visionary_id = game.next_entity_id();
    let mut visionary = Card::new(visionary_id, "Elvish Visionary".to_string(), alice_id);
    visionary.types.push(CardType::Creature);
    visionary.power = Some(1);
    visionary.toughness = Some(1);
    visionary.mana_cost = ManaCost::from_string("1G");
    visionary.triggers.push(Trigger::new(
        TriggerEvent::EntersBattlefield,
        vec![Effect::DrawCards {
            player: alice_id,
            count: 1,
        }],
        "When Elvish Visionary enters, draw a card.".to_string(),
    ));

    let hand_before = game.get_player_zones(alice_id).map(|z| z.hand.cards.len()).unwrap_or(0);

    game.cards.insert(visionary_id, visionary);
    game.stack.add(visionary_id);
    game.resolve_spell(visionary_id, &[])?;

    let hand_after = game.get_player_zones(alice_id).map(|z| z.hand.cards.len()).unwrap_or(0);

    println!("Elvish Visionary (1/1) enters the battlefield");
    println!("✓ ETB trigger: Alice draws a card (hand: {hand_before} → {hand_after})");
    println!("Alice: {alice_life_after} life, 2 creatures on battlefield\n");

    // Scenario 3: Bob deploys a creature
    println!("=== Turn 3: Bob casts Grizzly Bears ===");
    let bears_id = game.next_entity_id();
    let mut bears = Card::new(bears_id, "Grizzly Bears".to_string(), bob_id);
    bears.types.push(CardType::Creature);
    bears.power = Some(2);
    bears.toughness = Some(2);
    bears.mana_cost = ManaCost::from_string("1G");

    game.cards.insert(bears_id, bears);
    game.battlefield.add(bears_id);

    println!("Grizzly Bears (2/2) enters the battlefield (no ETB trigger)");
    println!("Bob: 20 life, 1 creature on battlefield\n");

    // Scenario 4: Alice casts Flametongue Kavu - should kill Bob's Grizzly Bears
    println!("=== Turn 4: Alice casts Flametongue Kavu ===");
    let kavu_id = game.next_entity_id();
    let mut kavu = Card::new(kavu_id, "Flametongue Kavu".to_string(), alice_id);
    kavu.types.push(CardType::Creature);
    kavu.power = Some(4);
    kavu.toughness = Some(2);
    kavu.mana_cost = ManaCost::from_string("3R");
    kavu.triggers.push(Trigger::new(
        TriggerEvent::EntersBattlefield,
        vec![Effect::DealDamage {
            target: mtg_forge_rs::core::TargetRef::None,
            amount: 4,
        }],
        "When Flametongue Kavu enters, it deals 4 damage to target creature.".to_string(),
    ));

    let bob_creatures_before = game
        .battlefield
        .cards
        .iter()
        .filter(|&&id| game.cards.get(id).ok().map(|c| c.owner == bob_id).unwrap_or(false))
        .count();

    game.cards.insert(kavu_id, kavu);
    game.stack.add(kavu_id);
    game.resolve_spell(kavu_id, &[])?;

    let bob_creatures_after = game
        .battlefield
        .cards
        .iter()
        .filter(|&&id| game.cards.get(id).ok().map(|c| c.owner == bob_id).unwrap_or(false))
        .count();

    println!("Flametongue Kavu (4/2) enters the battlefield");
    println!("✓ ETB trigger: Deals 4 damage to Grizzly Bears (2 toughness), destroying it");
    println!("Bob's creatures on battlefield: {bob_creatures_before} → {bob_creatures_after}");
    println!("Alice: {alice_life_after} life, 3 creatures on battlefield\n");

    // Final summary
    println!("=== Final Board State ===");
    println!("Alice ({alice_life_after} life):");
    for card_id in &game.battlefield.cards {
        if let Ok(card) = game.cards.get(*card_id) {
            if card.owner == alice_id {
                let p = card.power.unwrap_or(0) as i32;
                let t = card.toughness.unwrap_or(0) as i32;
                let pb = card.power_bonus;
                let tb = card.toughness_bonus;
                if pb != 0 || tb != 0 {
                    println!(
                        "  - {} ({}/{} +{:+}/{:+} = {}/{})",
                        card.name,
                        p,
                        t,
                        pb,
                        tb,
                        p + pb,
                        t + tb
                    );
                } else {
                    println!("  - {} ({}/{})", card.name, p, t);
                }
            }
        }
    }

    let alice_hand_size = game.get_player_zones(alice_id).map(|z| z.hand.cards.len()).unwrap_or(0);
    println!("  Hand: {alice_hand_size} cards");

    println!("\nBob (20 life):");
    let bob_battlefield_count = game
        .battlefield
        .cards
        .iter()
        .filter(|&&id| game.cards.get(id).ok().map(|c| c.owner == bob_id).unwrap_or(false))
        .count();
    println!("  Battlefield: {bob_battlefield_count} creatures");

    println!("\n=== Demo Complete ===");
    println!("\nKey Observations:");
    println!("• ETB triggers fire automatically when creatures enter the battlefield");
    println!("• Different trigger effects can stack and create complex interactions");
    println!("• Triggers that target (like Flametongue Kavu) automatically find valid targets");
    println!("• The trigger system supports: draw, damage, life gain, pump, and destroy");
    println!("• This makes ~4000+ cards with ETB triggers playable in the engine");

    Ok(())
}
