//! Lightning Bolt Game - Deck Loading Version
//!
//! Demonstrates game initialization from decks and mid-game scenarios.
//! Uses the AsyncCardDatabase and GameInitializer to set up a game state.

use mtg_forge_rs::core::PlayerId;
use mtg_forge_rs::loader::{prefetch_deck_cards, AsyncCardDatabase as CardDatabase, DeckLoader, GameInitializer};
use std::path::PathBuf;

#[tokio::main]
async fn main() {
    println!("=== MTG Forge - Lightning Bolt Deck Loading Demo ===\n");
    println!("Demonstrates:");
    println!("  - Loading cards from cardsfolder (async, on-demand)");
    println!("  - Initializing game from deck files");
    println!("  - Setting up mid-game scenarios");
    println!("  - Casting spells with proper state management\n");

    // Load the card database from cardsfolder
    let cardsfolder = PathBuf::from("cardsfolder");
    if !cardsfolder.exists() {
        eprintln!("Error: cardsfolder not found at {cardsfolder:?}");
        eprintln!("This example requires the cardsfolder to be present.");
        return;
    }

    // Create simple decks (just Mountains and Lightning Bolts)
    let deck_content = r#"
[Main]
20 Mountain
40 Lightning Bolt
"#;

    let deck = DeckLoader::parse(deck_content).expect("Failed to parse deck");
    println!("Deck loaded: {} total cards", deck.total_cards());
    println!("  - {} Mountains", 20);
    println!("  - {} Lightning Bolts\n", 40);

    // Create card database (lazy loading)
    let card_db = CardDatabase::new(cardsfolder);

    // Prefetch deck cards
    println!("Prefetching deck cards...");
    let start = std::time::Instant::now();
    let (count, _) = prefetch_deck_cards(&card_db, &deck)
        .await
        .expect("Failed to prefetch cards");
    let elapsed = start.elapsed();
    println!("Prefetched {} cards in {} ms\n", count, elapsed.as_millis());

    // Initialize the game with custom life totals
    let initializer = GameInitializer::new(&card_db);
    let mut game = initializer
        .init_game(
            "Alice".to_string(),
            &deck,
            "Bob".to_string(),
            &deck,
            20, // Starting life (we'll modify this)
        )
        .await
        .expect("Failed to initialize game");

    println!("Game initialized!");
    let alice = PlayerId::new(0);
    let bob = PlayerId::new(1);

    // Set up the mid-game scenario:
    // - Alice at 11 life
    // - Bob at 12 life
    // - Each player has a Mountain on the battlefield
    // - Alice has a Lightning Bolt in hand

    println!("\n=== Setting up mid-game scenario ===");

    // Set life totals
    game.players[alice.as_u32() as usize].life = 11;
    game.players[bob.as_u32() as usize].life = 12;
    println!("  Alice: 11 life");
    println!("  Bob: 12 life");

    // Move one Mountain from each player's library to battlefield
    for player_id in &[alice, bob] {
        // First, find the Mountain ID without holding a mutable borrow
        let mountain_id = if let Some(zones) = game.get_player_zones(*player_id) {
            zones
                .library
                .cards
                .iter()
                .find(|&&card_id| {
                    game.cards
                        .get(card_id)
                        .map(|c| {
                            c.name.as_str().eq_ignore_ascii_case("mountain") || c.name.as_str().starts_with("Mountain")
                        })
                        .unwrap_or(false)
                })
                .copied()
        } else {
            None
        };

        // Then mutate
        if let Some(mountain_id) = mountain_id {
            if let Some(zones) = game.get_player_zones_mut(*player_id) {
                zones.library.remove(mountain_id);
            }
            game.battlefield.add(mountain_id);

            let player_name = game.players[player_id.as_u32() as usize].name.clone();
            if let Ok(card) = game.cards.get(mountain_id) {
                println!("  {} has {} on battlefield", player_name, card.name);
            }
        }
    }

    // Move a Lightning Bolt from Alice's library to her hand
    // First find the bolt ID
    let bolt_id = if let Some(zones) = game.get_player_zones(alice) {
        zones
            .library
            .cards
            .iter()
            .find(|&&card_id| {
                game.cards
                    .get(card_id)
                    .map(|c| {
                        c.name.as_str().eq_ignore_ascii_case("lightning bolt")
                            || c.name.as_str().starts_with("Lightning Bolt")
                    })
                    .unwrap_or(false)
            })
            .copied()
    } else {
        None
    };

    // Then mutate
    if let Some(bolt_id) = bolt_id {
        if let Some(zones) = game.get_player_zones_mut(alice) {
            zones.library.remove(bolt_id);
            zones.hand.add(bolt_id);
        }
        println!("  Alice has Lightning Bolt in hand");

        // Note: Lightning Bolt now has its DealDamage effect automatically parsed
        // from the card definition, so we don't need to manually add it
    }

    println!("\n=== Alice's Turn - Casting Lightning Bolt ===\n");

    // Find Alice's untapped Mountain for mana
    let alice_mountain = game
        .battlefield
        .cards
        .iter()
        .find(|&&card_id| {
            game.cards
                .get(card_id)
                .map(|c| c.owner == alice && !c.tapped)
                .unwrap_or(false)
        })
        .copied();

    if let Some(mountain_id) = alice_mountain {
        println!("Alice taps Mountain for mana");
        if let Err(e) = game.tap_for_mana(alice, mountain_id) {
            println!("  Error: {e:?}");
        } else {
            let mana = game.players[alice.as_u32() as usize].mana_pool.red;
            println!("  Alice now has {mana} red mana\n");
        }
    }

    // Find Lightning Bolt in Alice's hand
    let bolt_id = if let Some(zones) = game.get_player_zones(alice) {
        zones
            .hand
            .cards
            .iter()
            .find(|&&card_id| {
                game.cards
                    .get(card_id)
                    .map(|c| c.name.as_str().contains("Lightning Bolt"))
                    .unwrap_or(false)
            })
            .copied()
    } else {
        None
    };

    if let Some(bolt_id) = bolt_id {
        println!("Alice casts Lightning Bolt targeting Bob");

        // Cast the spell
        if let Err(e) = game.cast_spell(alice, bolt_id, vec![]) {
            println!("  Error: {e:?}");
        } else {
            println!("  Lightning Bolt is on the stack");
            println!("  Bob: {} life", game.players[bob.as_u32() as usize].life);

            // Resolve the spell
            println!("\nLightning Bolt resolves:");
            if let Err(e) = game.resolve_spell(bolt_id, &[]) {
                println!("  Error: {e:?}");
            } else {
                let bob_life = game.players[bob.as_u32() as usize].life;
                println!("  Lightning Bolt deals 3 damage to Bob");
                println!("  Bob: {bob_life} life");

                // Verify the expected outcome
                assert_eq!(bob_life, 9, "Bob should be at 9 life (12 - 3)");
                println!("\n✓ Test passed: Bob's life correctly reduced to 9");
            }
        }
    }

    println!("\n=== Final Game State ===");
    println!("  Alice: {} life", game.players[alice.as_u32() as usize].life);
    println!("  Bob: {} life", game.players[bob.as_u32() as usize].life);
    println!("  Battlefield: {} cards", game.battlefield.cards.len());
    println!("  Stack: {} cards", game.stack.cards.len());

    if let Some(zones) = game.get_player_zones(alice) {
        println!("  Alice's graveyard: {} cards", zones.graveyard.cards.len());
    }

    println!("\n=== Demo Complete ===");
    println!("Key features demonstrated:");
    println!("  ✓ Loading cards from cardsfolder");
    println!("  ✓ Initializing game from deck definitions");
    println!("  ✓ Setting up custom game scenarios");
    println!("  ✓ Casting spells with mana payment");
    println!("  ✓ Resolving effects and updating life totals");
}
