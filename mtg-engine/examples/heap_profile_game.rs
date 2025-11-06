//! Heap Profiling Example
//!
//! Uses DHAT to profile heap allocations during a realistic game
//! Run with: cargo run --release --example heap_profile_game
//! View results at: https://nnethercote.github.io/dh_view/dh_view.html

#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

use mtg_forge_rs::loader::{prefetch_deck_cards, AsyncCardDatabase as CardDatabase, DeckLoader, GameInitializer};
use mtg_forge_rs::game::GameLoop;
use std::path::PathBuf;

#[tokio::main]
async fn main() {
    let _profiler = dhat::Profiler::new_heap();

    println!("=== DHAT Heap Profiling - Realistic Game ===\n");

    // Load card database
    let cardsfolder = PathBuf::from("cardsfolder");
    if !cardsfolder.exists() {
        eprintln!("ERROR: cardsfolder not found. Please run from repository root.");
        std::process::exit(1);
    }

    // Load the UR Burn deck - realistic with mana and spells
    let deck_path = PathBuf::from("decks/old_school2/ur_burn.dck");
    if !deck_path.exists() {
        eprintln!("ERROR: Deck file not found: {}", deck_path.display());
        std::process::exit(1);
    }

    println!("Loading deck: {}", deck_path.display());
    let deck_content = std::fs::read_to_string(&deck_path).expect("Failed to read deck file");
    let deck = DeckLoader::parse(&deck_content).expect("Failed to parse deck");

    println!("Deck configuration:");
    println!("  - {} total cards\n", deck.total_cards());

    // Create card database (lazy loading)
    let card_db = CardDatabase::new(cardsfolder);

    // Prefetch deck cards
    println!("Prefetching deck cards...");
    let start = std::time::Instant::now();
    match prefetch_deck_cards(&card_db, &deck).await {
        Ok((count, _)) => {
            let elapsed = start.elapsed();
            println!("Prefetched {} cards in {} ms\n", count, elapsed.as_millis());
        }
        Err(e) => {
            eprintln!("Error prefetching cards: {e}");
            eprintln!("Some cards may be missing from cardsfolder");
            // Continue anyway to get profiling data
        }
    }

    // Initialize game with the realistic deck
    let initializer = GameInitializer::new(&card_db);
    let mut game = initializer
        .init_game("Alice".to_string(), &deck, "Bob".to_string(), &deck, 20)
        .await
        .expect("Failed to initialize game");

    println!("Game initialized!");
    let players: Vec<_> = game.players.iter().map(|p| (p.id, p.name.to_string())).collect();
    println!("  - {}: 20 life", players[0].1);
    println!("  - {}: 20 life\n", players[1].1);

    // Seed the game RNG for determinism
    game.seed_rng(42);

    // Create AI controllers
    let mut alice_ai = mtg_forge_rs::game::random_controller::RandomController::with_seed(players[0].0, 42);
    let mut bob_ai = mtg_forge_rs::game::random_controller::RandomController::with_seed(players[1].0, 42);

    println!("=== Starting Game Loop ===\n");

    // Run the game for 20 turns to get meaningful profiling data
    let mut game_loop = GameLoop::new(&mut game).with_max_turns(20);

    let result = game_loop
        .run_game(&mut alice_ai, &mut bob_ai)
        .expect("Game loop failed");

    println!("\n=== Game Complete ===");
    println!("Turns played: {}", result.turns_played);
    println!("End reason: {:?}", result.end_reason);

    if let Some(winner_id) = result.winner {
        let winner_name = game.get_player(winner_id).map(|p| p.name.as_str()).unwrap_or("Unknown");
        println!("Winner: {winner_name}");
    } else {
        println!("Game ended in a draw");
    }

    println!("\nFinal life totals:");
    for player in game.players.iter() {
        println!("  - {}: {} life", player.name, player.life);
    }

    println!("\nFinal statistics:");
    println!("  - Total cards in game: {}", game.cards.len());
    println!("  - Cards on battlefield: {}", game.battlefield.cards.len());
    println!("  - Cards on stack: {}", game.stack.cards.len());

    println!("\n=== Profiling Complete ===");
    println!("DHAT results saved to: dhat-heap.json");
    println!("View at: https://nnethercote.github.io/dh_view/dh_view.html");
}
