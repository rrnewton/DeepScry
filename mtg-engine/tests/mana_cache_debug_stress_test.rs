//! Stress test for mana cache with debug verification
//!
//! This test runs games with oldschool decks and has debug mode enabled for the
//! ManaEngine. This verifies that the incremental mana source caching (using
//! event-driven updates) produces the same results as a from-scratch battlefield
//! scan on every mana query.
//!
//! This is the "from-scratch consistency" gold standard for incremental computing:
//! the incremental result must match the full recomputation result.

use mtg_forge_rs::{
    game::{random_controller::RandomController, GameLoop, VerbosityLevel},
    loader::{AsyncCardDatabase as CardDatabase, DeckLoader, GameInitializer},
    Result,
};
use std::path::PathBuf;

/// Run a small tournament of oldschool deck games with mana cache debug verification
#[tokio::test]
#[ignore] // TODO: Fix cache consistency bug - currently fails with capacity mismatch
async fn test_oldschool_tourney_mana_cache_debug() -> Result<()> {
    println!("\n=== Mana Cache Debug Stress Test ===\n");
    println!("Running oldschool deck games with from-scratch consistency verification");
    println!("Every mana query will be verified against full battlefield scan\n");

    // Use a subset of oldschool decks for the stress test
    // Tests run from project root, so paths are relative to mtg-forge-rs-fedora/
    let deck_paths = vec![
        "../decks/old_school/01_rogue_rogerbrand.dck",
        "../decks/old_school/02_thedeck_peterschnidrig.dck",
        "../decks/old_school/03_robots_jesseisbak.dck",
        "../decks/old_school/05_mono_black_rogerbrand.dck",
    ];

    // Load decks
    println!("Loading decks...");
    let mut decks = Vec::new();
    for deck_path in &deck_paths {
        let path = PathBuf::from(deck_path);
        let deck = DeckLoader::load_from_file(&path)?;
        println!("  {}: {} cards", deck_path, deck.total_cards());
        decks.push((path, deck));
    }
    println!();

    // Load card database
    println!("Loading card database...");
    let cardsfolder = PathBuf::from("../cardsfolder");
    let card_db = CardDatabase::new(cardsfolder);

    let start = std::time::Instant::now();
    let mut all_card_names = std::collections::HashSet::new();
    for (_, deck) in &decks {
        all_card_names.extend(deck.unique_card_names());
    }
    let card_names: Vec<_> = all_card_names.into_iter().collect();
    let (count, _) = card_db.load_cards(&card_names).await?;
    let duration = start.elapsed();
    println!("  Loaded {} cards in {:.2}ms\n", count, duration.as_secs_f64() * 1000.0);

    // Run a small tournament (all matchups)
    let num_games = 20; // Run 20 games total
    println!("Running {} games with mana cache debug verification...\n", num_games);

    let seed = 42u64;
    let mut games_played = 0;
    let mut total_turns = 0;

    // Run games with different deck combinations
    for game_idx in 0..num_games {
        use rand::Rng;
        use rand::SeedableRng;

        // Select decks deterministically
        let deck_rng_seed = seed.wrapping_add(game_idx as u64);
        let mut deck_rng = rand_xoshiro::Xoshiro256PlusPlus::seed_from_u64(deck_rng_seed);

        let deck1_idx = deck_rng.gen_range(0..decks.len());
        let deck2_idx = deck_rng.gen_range(0..decks.len());

        let (deck1_path, deck1) = &decks[deck1_idx];
        let (deck2_path, deck2) = &decks[deck2_idx];

        // Initialize game
        let game_init = GameInitializer::new(&card_db);
        let mut game = game_init
            .init_game("Player 1".to_string(), deck1, "Player 2".to_string(), deck2, 20)
            .await?;

        // Seed the game RNG
        let game_seed = seed.wrapping_add((game_idx as u64).wrapping_mul(0x9E3779B97F4A7C15));
        game.seed_rng(game_seed);

        // Get player IDs
        let p1_id = game.get_player_by_idx(0).expect("Should have player 1").id;
        let p2_id = game.get_player_by_idx(1).expect("Should have player 2").id;

        // Derive controller seeds
        let p1_seed = game_seed.wrapping_add(0x1234_5678_9ABC_DEF0);
        let p2_seed = game_seed.wrapping_add(0xFEDC_BA98_7654_3210);

        // Create controllers
        let mut controller1 = RandomController::with_seed(p1_id, p1_seed);
        let mut controller2 = RandomController::with_seed(p2_id, p2_seed);

        // Create game loop with DEBUG MODE ENABLED for ManaEngine
        // This will verify every mana query against from-scratch computation
        let mut game_loop = GameLoop::new(&mut game)
            .with_verbosity(VerbosityLevel::Silent)
            .with_mana_debug_verification();

        match game_loop.run_game(&mut controller1, &mut controller2) {
            Ok(_result) => {
                games_played += 1;
                total_turns += game.turn.turn_number;

                // Print progress
                if (game_idx + 1) % 5 == 0 {
                    println!(
                        "  Completed {}/{} games (avg {:.1} turns/game)",
                        game_idx + 1,
                        num_games,
                        total_turns as f64 / games_played as f64
                    );
                }

                // If we get here, the mana cache verification passed for the entire game!
                // Any mismatch would have panicked inside verify_incremental_correctness()
            }
            Err(e) => {
                // Report error with reproduction information
                eprintln!("\n!!! Game {} failed !!!", game_idx);
                eprintln!("Deck 1: {}", deck1_path.display());
                eprintln!("Deck 2: {}", deck2_path.display());
                eprintln!("Game seed: {}", game_seed);
                eprintln!("P1 seed: {}", p1_seed);
                eprintln!("P2 seed: {}", p2_seed);
                eprintln!("Error: {}", e);
                eprintln!("\nReproduce with:");
                eprintln!(
                    "  cargo run --release --bin mtg -- tui --p1 random --p2 random --seed {} \"{}\" \"{}\"",
                    game_seed,
                    deck1_path.display(),
                    deck2_path.display()
                );
                return Err(e);
            }
        }
    }

    println!("\n=== Test Complete ===");
    println!("Total games: {}", games_played);
    println!("Average turns: {:.1}", total_turns as f64 / games_played as f64);
    println!("\n✅ All games passed mana cache debug verification!");
    println!("   Every mana query was verified against from-scratch computation.");

    Ok(())
}
