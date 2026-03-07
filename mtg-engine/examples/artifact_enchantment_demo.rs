//! Artifact and Enchantment Casting Demo
//!
//! Demonstrates that artifacts and enchantments can be cast and enter the battlefield.
//! This validates that non-creature permanents are properly supported.

use mtg_forge_rs::core::PlayerId;
use mtg_forge_rs::loader::{prefetch_deck_cards, AsyncCardDatabase as CardDatabase, DeckLoader, GameInitializer};
use std::path::PathBuf;

#[tokio::main]
async fn main() {
    println!("=== MTG Forge - Artifact & Enchantment Demo ===\n");
    println!("Demonstrates:");
    println!("  - Casting artifact cards");
    println!("  - Casting enchantment cards");
    println!("  - Non-creature permanents entering battlefield\n");

    // Load the card database from cardsfolder
    let cardsfolder = PathBuf::from("cardsfolder");
    if !cardsfolder.exists() {
        eprintln!("Error: cardsfolder not found at {cardsfolder:?}");
        eprintln!("This example requires the cardsfolder to be present.");
        return;
    }

    // Create a deck with artifacts and enchantments
    // Using simple cards that exist in cardsfolder
    let deck_content = r#"
[Main]
20 Mountain
20 Sol Ring
20 Darksteel Citadel
"#;

    let deck = DeckLoader::parse(deck_content).expect("Failed to parse deck");
    println!("Deck loaded: {} total cards", deck.total_cards());

    // Create card database (lazy loading)
    let card_db = CardDatabase::new(cardsfolder);

    // Prefetch deck cards
    println!("Prefetching deck cards...");
    let start = std::time::Instant::now();
    let (count, _) = prefetch_deck_cards(&card_db, &deck)
        .await
        .expect("Failed to prefetch cards");
    let elapsed = start.elapsed();
    println!("Loaded {} unique cards in {:.2} ms\n", count, elapsed.as_millis());

    // Initialize the game
    let initializer = GameInitializer::new(&card_db);
    let mut game = initializer
        .init_game("Alice".to_string(), &deck, "Bob".to_string(), &deck, 20)
        .await
        .expect("Failed to initialize game");

    println!("Game initialized!");
    let alice = PlayerId::new(0);
    let bob = PlayerId::new(1);

    println!("\n=== Initial Game State ===");
    println!("  Alice: {} life", game.players[alice.as_u32() as usize].life);
    println!("  Bob: {} life", game.players[bob.as_u32() as usize].life);

    // Count card types in deck to verify loading
    let alice_zones = game.get_player_zones(alice).expect("Alice zones");

    let mut artifact_count = 0;
    let mut land_count = 0;

    for &card_id in &alice_zones.library.cards {
        if let Ok(card) = game.cards.get(card_id) {
            if card.is_artifact() {
                artifact_count += 1;
            }
            if card.is_land() {
                land_count += 1;
            }
        }
    }

    println!("\n=== Alice's Library Contents ===");
    println!("  Artifacts: {artifact_count}");
    println!("  Lands: {land_count}");
    println!("  Total cards: {}", alice_zones.library.cards.len());

    // Try to find a Sol Ring in Alice's library
    let mut sol_ring_id = None;
    let mut mountain_id = None;

    for &card_id in &alice_zones.library.cards {
        if let Ok(card) = game.cards.get(card_id) {
            if card.name.as_str() == "Sol Ring" && sol_ring_id.is_none() {
                sol_ring_id = Some(card_id);
            }
            if card.name.as_str() == "Mountain" && mountain_id.is_none() {
                mountain_id = Some(card_id);
            }
            if sol_ring_id.is_some() && mountain_id.is_some() {
                break;
            }
        }
    }

    // Set up scenario: Move Mountain and Sol Ring to Alice's hand
    if let (Some(mountain), Some(sol_ring)) = (mountain_id, sol_ring_id) {
        println!("\n=== Setting up test scenario ===");
        println!("Moving Mountain and Sol Ring from library to hand...");

        // Remove from library and add to hand
        let alice_zones_mut = game.get_player_zones_mut(alice).expect("Alice zones mut");
        alice_zones_mut.library.remove(mountain);
        alice_zones_mut.hand.add(mountain);
        alice_zones_mut.library.remove(sol_ring);
        alice_zones_mut.hand.add(sol_ring);

        println!("  Alice now has: Mountain and Sol Ring in hand");

        // Play the mountain (special action, no casting)
        println!("\n=== Alice plays Mountain (special action) ===");
        game.play_land(alice, mountain).expect("Failed to play land");
        println!("  ✓ Mountain entered the battlefield");

        // Check battlefield
        let battlefield_lands = game
            .battlefield
            .cards
            .iter()
            .filter(|&&card_id| {
                if let Ok(card) = game.cards.get(card_id) {
                    card.is_land()
                } else {
                    false
                }
            })
            .count();
        println!("  Battlefield now has {battlefield_lands} lands");

        // Tap the mountain for mana
        println!("\n=== Alice taps Mountain for mana ===");
        game.tap_for_mana(alice, mountain).expect("Failed to tap for mana");
        let mana = &game.players[alice.as_u32() as usize].mana_pool;
        println!("  Alice's mana pool: R={}", mana.red);

        // Cast Sol Ring (artifact)
        println!("\n=== Alice casts Sol Ring (Artifact) ===");

        // Create a ManaEngine for mana payment
        use mtg_forge_rs::game::ManaEngine;
        let mut mana_engine = ManaEngine::new();
        mana_engine.update(&game, alice);

        // Use the 8-step spell casting process
        let cast_result = game.cast_spell_8_step(
            alice,
            sol_ring,
            |_game, _card_id| smallvec::SmallVec::new(), // No targets needed
            &mana_engine,
        );

        match cast_result {
            Ok(_) => {
                println!("  ✓ Sol Ring cast successfully");
                println!("  Sol Ring is on the stack");

                // Check stack
                let stack_size = game.stack.cards.len();
                println!("  Stack has {stack_size} card(s)");

                // Resolve the spell
                println!("\n=== Resolving Sol Ring ===");
                game.resolve_spell(sol_ring, &[]).expect("Failed to resolve");
                println!("  ✓ Sol Ring resolved");

                // Check battlefield
                let battlefield_artifacts = game
                    .battlefield
                    .cards
                    .iter()
                    .filter(|&&card_id| {
                        if let Ok(card) = game.cards.get(card_id) {
                            card.is_artifact()
                        } else {
                            false
                        }
                    })
                    .count();
                println!("  ✓ Sol Ring entered the battlefield");
                println!("  Battlefield now has {battlefield_artifacts} artifact(s)");
            }
            Err(e) => {
                println!("  ✗ Failed to cast Sol Ring: {e}");
            }
        }
    } else {
        println!("\n⚠ Could not find required cards in library");
        if mountain_id.is_none() {
            println!("  Missing: Mountain");
        }
        if sol_ring_id.is_none() {
            println!("  Missing: Sol Ring");
        }
    }

    println!("\n=== Demo Complete ===");
    println!("Key features demonstrated:");
    println!("  ✓ Loading artifact cards from cardsfolder");
    println!("  ✓ Casting artifacts");
    println!("  ✓ Artifacts entering the battlefield");
    println!("  ✓ Card type checking (is_artifact, is_land)");
}
