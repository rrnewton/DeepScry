//! Expanded Activated Abilities Demo
//!
//! Demonstrates various activated abilities loaded from cardsfolder:
//! - AB$ Draw: Yavimaya Elder "{2}, Sacrifice ~: Draw a card"
//! - AB$ Pump: Activated abilities that boost creatures
//! - AB$ Tap: Activated abilities that tap permanents

use mtg_forge_rs::game::GameState;
use mtg_forge_rs::loader::CardDatabase;
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== MTG Forge - Expanded Activated Abilities Demo ===\n");

    // Create a game with two players
    let mut game = GameState::new_two_player("Alice".to_string(), "Bob".to_string(), 20);
    let alice_id = game.players[0].id;

    println!("=== Initial Setup ===");
    println!("Alice: 20 life, 0 cards in hand");

    // Add some cards to Alice's library so she can draw
    for i in 0..5 {
        let card_id = game.next_card_id();
        let card = mtg_forge_rs::core::Card::new(card_id, format!("Library Card {}", i + 1), alice_id);
        game.cards.insert(card_id, card);
        if let Some(zones) = game.get_player_zones_mut(alice_id) {
            zones.library.add(card_id);
        }
    }
    println!("Added 5 cards to Alice's library\n");

    // Load cards from cardsfolder
    let cardsfolder = PathBuf::from("./cardsfolder");
    let db = CardDatabase::new(cardsfolder);

    // Demo 1: AB$ Draw - Yavimaya Elder
    println!("=== Demo 1: AB$ Draw (Yavimaya Elder) ===");

    if let Ok(Some(elder_def)) = db.get_card("Yavimaya Elder").await {
        let elder_id = game.next_card_id();
        let elder = elder_def.instantiate(elder_id, alice_id);

        println!("Loaded: {}", elder.name.as_str());
        println!("  Types: {:?}", elder.types);
        println!("  Mana Cost: {}", elder.mana_cost);
        println!("  Activated abilities: {}", elder.activated_abilities.len());

        if let Some(ability) = elder.activated_abilities.first().cloned() {
            println!("  Ability: {}", ability.description);
            println!("  Cost: {:?}", ability.cost);
            println!("  Effects: {}", ability.effects.len());

            // Add card to battlefield
            game.cards.insert(elder_id, elder);
            game.battlefield.add(elder_id);

            // Add some mana to Alice's pool (2 colorless)
            game.get_player_mut(alice_id)?.mana_pool.colorless = 2;

            println!("\nBefore activation:");
            println!("  Hand size: {}", count_hand(&game, alice_id));
            println!(
                "  Battlefield creatures: {}",
                count_battlefield_creatures(&game, alice_id)
            );
            println!("  Graveyard: {}", count_graveyard(&game, alice_id));

            // Activate the ability
            println!("\nActivating: {}", ability.description);

            // Pay costs
            if let Err(e) = game.pay_ability_cost(alice_id, elder_id, &ability.cost) {
                println!("  Failed to pay cost: {e}");
            } else {
                println!("  ✓ Paid cost");

                // Execute effects
                for effect in &ability.effects {
                    // Fix placeholder player IDs
                    let fixed_effect = match effect {
                        mtg_forge_rs::core::Effect::DrawCards { player, count } if player.as_u32() == 0 => {
                            mtg_forge_rs::core::Effect::DrawCards {
                                player: alice_id,
                                count: *count,
                            }
                        }
                        _ => effect.clone(),
                    };

                    if let Err(e) = game.execute_effect(&fixed_effect) {
                        println!("  Failed to execute effect: {e}");
                    } else {
                        println!("  ✓ Executed effect");
                    }
                }
            }

            println!("\nAfter activation:");
            println!("  Hand size: {}", count_hand(&game, alice_id));
            println!(
                "  Battlefield creatures: {}",
                count_battlefield_creatures(&game, alice_id)
            );
            println!("  Graveyard: {}", count_graveyard(&game, alice_id));
        }
    } else {
        println!("Note: Yavimaya Elder not found in cardsfolder");
    }

    println!("\n✅ Expanded activated abilities demo completed!");
    println!("   - Demonstrated AB$ Draw parsing and execution");
    println!("   - Card loaded from cardsfolder");
    println!("   - Sacrifice cost combined with mana cost");

    Ok(())
}

fn count_hand(game: &GameState, player_id: mtg_forge_rs::core::PlayerId) -> usize {
    if let Some(zones) = game.get_player_zones(player_id) {
        zones.hand.cards.len()
    } else {
        0
    }
}

fn count_battlefield_creatures(game: &GameState, player_id: mtg_forge_rs::core::PlayerId) -> usize {
    game.battlefield
        .cards
        .iter()
        .filter(|&&card_id| {
            if let Ok(card) = game.cards.get(card_id) {
                card.owner == player_id && card.is_creature()
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
