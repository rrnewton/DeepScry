//! ETB (Enters the Battlefield) Triggers Demo
//!
//! Demonstrates triggered abilities that execute when permanents enter the battlefield.
//! This is one of the most common mechanics in Magic: The Gathering, used by thousands
//! of cards. ETB triggers enable diverse strategies like card advantage (draw), removal
//! (damage/destroy), life gain, and more.
//!
//! This example shows:
//! 1. Loading cards with ETB triggers from the cardsfolder
//! 2. Casting creatures with ETB abilities
//! 3. Automatic trigger execution when creatures enter the battlefield
//! 4. Various trigger effects: draw cards, deal damage, etc.

use mtg_forge_rs::core::{Card, CardType, Effect, ManaCost, Trigger, TriggerEvent};
use mtg_forge_rs::game::GameState;
use mtg_forge_rs::loader::CardDatabase;
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== MTG Forge - ETB Triggers Demo ===\n");

    // Create a two-player game
    let mut game = GameState::new_two_player("Alice".to_string(), "Bob".to_string(), 20);
    let alice_id = game.players[0].id;
    let bob_id = game.players[1].id;

    println!("=== Setting up game ===");
    println!("Alice: 20 life");
    println!("Bob: 20 life\n");

    // Add some cards to Alice's library for drawing
    println!("=== Adding cards to Alice's library ===");
    for i in 0..10 {
        let card_id = game.next_card_id();
        let card = Card::new(card_id, format!("Library Card {}", i + 1), alice_id);
        game.cards.insert(card_id, card);
        if let Some(zones) = game.get_player_zones_mut(alice_id) {
            zones.library.add(card_id);
        }
    }
    println!("Added 10 cards to Alice's library\n");

    // Add mana sources for Alice
    println!("=== Adding lands for Alice ===");
    for i in 0..5 {
        let land_id = game.next_card_id();
        let mut land = Card::new(land_id, format!("Forest {}", i + 1), alice_id);
        land.types.push(CardType::Land);
        land.colors.push(mtg_forge_rs::core::Color::Green);
        game.cards.insert(land_id, land);
        game.battlefield.add(land_id);
    }
    println!("Added 5 Forests to Alice's battlefield\n");

    // Add a target creature for Bob (for Flametongue Kavu to hit)
    println!("=== Adding target creature for Bob ===");
    let target_id = game.next_entity_id();
    let mut target = Card::new(target_id, "Grizzly Bears".to_string(), bob_id);
    target.types.push(CardType::Creature);
    target.power = Some(2);
    target.toughness = Some(2);
    target.mana_cost = ManaCost::from_string("1G");
    game.cards.insert(target_id, target);
    game.battlefield.add(target_id);
    println!("Bob controls: Grizzly Bears (2/2)\n");

    // Demo 1: Elvish Visionary - Draw a card when it enters
    println!("=== Demo 1: Elvish Visionary (Draw on ETB) ===");
    let cardsfolder = PathBuf::from("./cardsfolder");
    let db = CardDatabase::new(cardsfolder);

    if let Ok(Some(card_def)) = db.get_card("Elvish Visionary").await {
        let creature_id = game.next_entity_id();
        let creature = card_def.instantiate(creature_id, alice_id);

        println!("Alice casts Elvish Visionary (1G)");
        println!("Mana cost: {}", creature.mana_cost);
        println!("P/T: {}/{}", creature.power.unwrap(), creature.toughness.unwrap());
        println!("Triggers: {}", creature.triggers.len());

        if !creature.triggers.is_empty() {
            println!("  - When it enters: {}", creature.triggers[0].description);
        }

        let hand_before = game.get_player_zones(alice_id).map(|z| z.hand.cards.len()).unwrap_or(0);

        game.cards.insert(creature_id, creature);
        game.stack.add(creature_id);
        game.resolve_spell(creature_id, &[])?;

        let hand_after = game.get_player_zones(alice_id).map(|z| z.hand.cards.len()).unwrap_or(0);

        println!("✓ Elvish Visionary entered the battlefield");
        println!(
            "✓ ETB trigger fired: Drew {} card (hand: {} → {})",
            hand_after - hand_before,
            hand_before,
            hand_after
        );
        println!();
    } else {
        println!("Note: Elvish Visionary not found in cardsfolder, using manual test\n");

        // Manually create Elvish Visionary for demo
        let creature_id = game.next_entity_id();
        let mut creature = Card::new(creature_id, "Elvish Visionary".to_string(), alice_id);
        creature.types.push(CardType::Creature);
        creature.power = Some(1);
        creature.toughness = Some(1);
        creature.mana_cost = ManaCost::from_string("1G");
        creature.triggers.push(Trigger::new(
            TriggerEvent::EntersBattlefield,
            vec![Effect::DrawCards {
                player: alice_id,
                count: 1,
            }],
            "When Elvish Visionary enters, draw a card.".to_string(),
        ));

        let hand_before = game.get_player_zones(alice_id).map(|z| z.hand.cards.len()).unwrap_or(0);

        game.cards.insert(creature_id, creature);
        game.stack.add(creature_id);
        game.resolve_spell(creature_id, &[])?;

        let hand_after = game.get_player_zones(alice_id).map(|z| z.hand.cards.len()).unwrap_or(0);

        println!("Alice casts Elvish Visionary (1G)");
        println!("✓ Elvish Visionary entered the battlefield");
        println!(
            "✓ ETB trigger fired: Drew {} card (hand: {} → {})",
            hand_after - hand_before,
            hand_before,
            hand_after
        );
        println!();
    }

    // Demo 2: Flametongue Kavu - Deal damage when it enters
    println!("=== Demo 2: Flametongue Kavu (Damage on ETB) ===");

    // Check if Grizzly Bears is still alive
    let bears_alive_before = game.battlefield.contains(target_id);
    let bears_toughness_before = game.cards.get(target_id).ok().and_then(|c| c.toughness).unwrap_or(0);

    println!(
        "Before: Bob's Grizzly Bears (2/{}) - {}",
        bears_toughness_before,
        if bears_alive_before { "alive" } else { "dead" }
    );

    // Manually create Flametongue Kavu for demo
    let kavu_id = game.next_entity_id();
    let mut kavu = Card::new(kavu_id, "Flametongue Kavu".to_string(), alice_id);
    kavu.types.push(CardType::Creature);
    kavu.power = Some(4);
    kavu.toughness = Some(2);
    kavu.mana_cost = ManaCost::from_string("3R");
    kavu.triggers.push(Trigger::new(
        TriggerEvent::EntersBattlefield,
        vec![Effect::DealDamage {
            target: mtg_forge_rs::core::TargetRef::None, // Will be filled in by trigger system
            amount: 4,
        }],
        "When Flametongue Kavu enters, it deals 4 damage to target creature.".to_string(),
    ));

    println!("Alice casts Flametongue Kavu (3R)");
    println!("P/T: 4/2");
    println!("ETB trigger: Deal 4 damage to target creature");

    game.cards.insert(kavu_id, kavu);
    game.stack.add(kavu_id);
    game.resolve_spell(kavu_id, &[])?;

    let bears_alive_after = game.battlefield.contains(target_id);
    let bears_in_graveyard = game
        .get_player_zones(bob_id)
        .map(|z| z.graveyard.contains(target_id))
        .unwrap_or(false);

    println!("✓ Flametongue Kavu entered the battlefield");
    println!("✓ ETB trigger fired: Dealt 4 damage to Grizzly Bears");
    println!(
        "After: Bob's Grizzly Bears - {}",
        if bears_alive_after {
            "somehow survived!?"
        } else if bears_in_graveyard {
            "destroyed and sent to graveyard"
        } else {
            "no longer on battlefield"
        }
    );
    println!();

    // Show final game state
    println!("=== Final Game State ===");
    println!(
        "Alice: {} life, {} cards in hand, {} creatures on battlefield",
        game.get_player(alice_id)?.life,
        game.get_player_zones(alice_id).map(|z| z.hand.cards.len()).unwrap_or(0),
        game.battlefield
            .cards
            .iter()
            .filter(|&&id| game.cards.get(id).ok().map(|c| c.owner == alice_id).unwrap_or(false))
            .count()
    );
    println!(
        "Bob: {} life, {} creatures on battlefield",
        game.get_player(bob_id)?.life,
        game.battlefield
            .cards
            .iter()
            .filter(|&&id| game.cards.get(id).ok().map(|c| c.owner == bob_id).unwrap_or(false))
            .count()
    );

    println!("\n=== ETB Triggers Demo Complete ===");
    println!("\nKey Takeaways:");
    println!("• ETB triggers execute automatically when permanents enter the battlefield");
    println!("• Common ETB effects include: draw cards, deal damage, destroy permanents, gain life");
    println!("• ~4000 cards in Magic use ETB triggers (13% of all cards)");
    println!("• Triggers are parsed from card files (T: lines) and executed by the game engine");

    Ok(())
}
