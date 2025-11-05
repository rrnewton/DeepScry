//! Sacrifice Cost Demo
//!
//! Demonstrates activated abilities with sacrifice costs using Zuran Orb.
//! Zuran Orb: "Sacrifice a land: You gain 2 life."

use mtg_forge_rs::game::GameState;
use mtg_forge_rs::loader::CardDatabase;
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== MTG Forge - Sacrifice Cost Demo ===\n");
    println!("This demo shows sacrifice costs for activated abilities:");
    println!("  Zuran Orb: Sacrifice a land: You gain 2 life\n");

    // Create a game with two players
    let mut game = GameState::new_two_player("Alice".to_string(), "Bob".to_string(), 20);
    let alice_id = game.players[0].id;

    println!("=== Initial Setup ===");
    println!("Alice: 20 life\n");

    // Load Zuran Orb from cardsfolder
    println!("=== Loading Zuran Orb from Cardsfolder ===");
    let cardsfolder = PathBuf::from("./cardsfolder");
    let db = CardDatabase::new(cardsfolder);

    let zuran_orb_def = db
        .get_card("Zuran Orb")
        .await?
        .unwrap_or_else(|| panic!("Zuran Orb not found in cardsfolder"));

    let zuran_orb_id = game.next_card_id();
    let zuran_orb = zuran_orb_def.instantiate(zuran_orb_id, alice_id);

    println!("Loaded: {}", zuran_orb.name.as_str());
    println!("  Types: {:?}", zuran_orb.types);
    println!("  Activated abilities: {}", zuran_orb.activated_abilities.len());

    if let Some(ability) = zuran_orb.activated_abilities.first() {
        println!("  Ability: {}", ability.description);
        println!("  Cost: {:?}", ability.cost);
        println!("  Effects: {} effect(s)\n", ability.effects.len());
    }

    // Add Zuran Orb to battlefield
    game.cards.insert(zuran_orb_id, zuran_orb);
    game.battlefield.add(zuran_orb_id);

    // Add some basic lands to Alice's battlefield
    println!("=== Adding Lands to Battlefield ===");
    for i in 0..3 {
        let land_name = match i {
            0 => "Forest",
            1 => "Plains",
            2 => "Island",
            _ => unreachable!(),
        };

        let land_def = db
            .get_card(land_name)
            .await?
            .unwrap_or_else(|| panic!("{land_name} not found in cardsfolder"));

        let land_id = game.next_card_id();
        let land = land_def.instantiate(land_id, alice_id);
        game.cards.insert(land_id, land);
        game.battlefield.add(land_id);

        println!("  Added {land_name} to battlefield");
    }

    println!("\n=== Current State ===");
    println!("Alice's life: {}", game.get_player(alice_id)?.life);
    println!("Lands on battlefield: {}", count_lands(&game, alice_id));
    println!("Cards in graveyard: {}", count_graveyard(&game, alice_id));

    // Activate Zuran Orb's ability 3 times
    println!("\n=== Activating Zuran Orb ===");

    for activation in 1..=3 {
        println!("\nActivation {activation}:");

        let lands_before = count_lands(&game, alice_id);
        let life_before = game.get_player(alice_id)?.life;
        let graveyard_before = count_graveyard(&game, alice_id);

        // Get the ability
        let ability = game
            .cards
            .get(zuran_orb_id)?
            .activated_abilities
            .first()
            .cloned()
            .ok_or("Zuran Orb has no abilities")?;

        // Pay the cost (sacrifice a land)
        println!("  Paying cost: {:?}", ability.cost);
        game.pay_ability_cost(alice_id, zuran_orb_id, &ability.cost)?;

        // Execute the effect (gain 2 life)
        for effect in &ability.effects {
            game.execute_effect(effect)?;
        }

        let lands_after = count_lands(&game, alice_id);
        let life_after = game.get_player(alice_id)?.life;
        let graveyard_after = count_graveyard(&game, alice_id);

        println!("  ✓ Sacrificed 1 land");
        println!("    Lands: {lands_before} → {lands_after}");
        println!("    Graveyard: {graveyard_before} → {graveyard_after}");
        println!("  ✓ Gained 2 life");
        println!("    Life: {life_before} → {life_after}");
    }

    println!("\n=== Final State ===");
    println!("Alice's life: {}", game.get_player(alice_id)?.life);
    println!("Lands on battlefield: {}", count_lands(&game, alice_id));
    println!("Cards in graveyard: {}", count_graveyard(&game, alice_id));

    println!("\n✅ Sacrifice cost demo completed successfully!");
    println!("   - Loaded Zuran Orb from cardsfolder");
    println!("   - Activated ability with Sac<1/Land> cost");
    println!("   - Sacrificed 3 lands, gained 6 life total");
    println!("   - All sacrificed lands moved to graveyard");

    Ok(())
}

fn count_lands(game: &GameState, player_id: mtg_forge_rs::core::PlayerId) -> usize {
    game.battlefield
        .cards
        .iter()
        .filter(|&&card_id| {
            if let Ok(card) = game.cards.get(card_id) {
                card.owner == player_id && card.is_land()
            } else {
                false
            }
        })
        .count()
}

fn count_graveyard(game: &GameState, player_id: mtg_forge_rs::core::PlayerId) -> usize {
    if let Some(zones) = game.get_player_zones(player_id) {
        zones.graveyard.cards.len()
    } else {
        0
    }
}
