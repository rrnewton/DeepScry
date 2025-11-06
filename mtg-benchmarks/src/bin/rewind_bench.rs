//! Standalone binary for running rewind + play again benchmark
//!
//! This bypasses Criterion and runs a single batch directly, printing metrics.
//!
//! Usage:
//!   cargo run --release --package mtg-benchmarks --bin rewind_bench [batch_size]
//!
//! Default batch size: 1000 games

use mtg_forge_rs::game::{random_controller::RandomController, GameLoop, GameState, VerbosityLevel};
use mtg_forge_rs::loader::{
    prefetch_deck_cards, AsyncCardDatabase as CardDatabase, DeckList, DeckLoader, GameInitializer,
};
use std::path::PathBuf;
use std::time::Instant;
use tokio::runtime::Runtime;

// Import allocator for stats tracking
use stats_alloc::{Region, StatsAlloc, INSTRUMENTED_SYSTEM};
use std::alloc::System;

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

const BASELINE_DECK_PATH: &str = "decks/old_school/03_robots_jesseisbak.dck";

/// Game outcome for win rate tracking
#[derive(Debug, Clone, Copy)]
enum GameOutcome {
    Player1Win,
    Player2Win,
}

/// Setup data for benchmark
struct BenchmarkSetup {
    card_db: CardDatabase,
    deck: DeckList,
    runtime: Runtime,
}

impl BenchmarkSetup {
    fn load(deck_path: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let runtime = Runtime::new()?;
        let cardsfolder = PathBuf::from("cardsfolder");
        let card_db = CardDatabase::new(cardsfolder);
        let deck = DeckLoader::load_from_file(&PathBuf::from(deck_path))?;

        runtime.block_on(async { prefetch_deck_cards(&card_db, &deck).await })?;

        Ok(BenchmarkSetup { card_db, deck, runtime })
    }
}

/// Create a mid-game state by playing to halfway point
fn create_midgame_state(setup: &BenchmarkSetup, seed: u64) -> (GameState, usize) {
    let game_init = GameInitializer::new(&setup.card_db);
    let mut game = setup
        .runtime
        .block_on(async {
            game_init
                .init_game(
                    "Player 1".to_string(),
                    &setup.deck,
                    "Player 2".to_string(),
                    &setup.deck,
                    20,
                )
                .await
        })
        .expect("Failed to initialize game");

    game.seed_rng(seed);

    // Play the game once to build the undo log
    {
        let (p1_id, p2_id) = {
            let mut players_iter = game.players.iter().map(|p| p.id);
            (
                players_iter.next().expect("Should have player 1"),
                players_iter.next().expect("Should have player 2"),
            )
        };

        let mut controller1 = RandomController::with_seed(p1_id, seed);
        let mut controller2 = RandomController::with_seed(p2_id, seed);

        let mut game_loop = GameLoop::new(&mut game).with_verbosity(VerbosityLevel::Silent);
        let _ = game_loop
            .run_game(&mut controller1, &mut controller2)
            .expect("Initial game should complete");
    }

    let total_actions = game.undo_log.len();
    let rewind_target = total_actions / 2;

    // Rewind to the target position
    while game.undo_log.len() > rewind_target {
        game.undo().expect("Undo should succeed");
    }

    // Clear the undo log at midpoint
    game.undo_log.clear();

    (game, total_actions)
}

/// Execute a single game from midpoint to end and rewind back
fn execute_single_game(game: &mut GameState, seed: u64) -> (u32, usize, GameOutcome) {
    game.seed_rng(seed);

    // Record starting state
    let start_turn = game.turn.turn_number;
    let start_undo_size = game.undo_log.len();

    let (p1_id, p2_id) = {
        let mut players_iter = game.players.iter().map(|p| p.id);
        (
            players_iter.next().expect("Should have player 1"),
            players_iter.next().expect("Should have player 2"),
        )
    };

    let mut controller1 = RandomController::with_seed(p1_id, seed);
    let mut controller2 = RandomController::with_seed(p2_id, seed);

    let mut game_loop = GameLoop::new(game).with_verbosity(VerbosityLevel::Silent);
    let result = game_loop
        .run_game(&mut controller1, &mut controller2)
        .expect("Game should complete");

    // Capture metrics BEFORE rewinding
    let end_turn = game.turn.turn_number;
    let turns_played = end_turn.saturating_sub(start_turn);
    let actions_played = game.undo_log.len() - start_undo_size;

    // Determine winner
    let outcome = if result.winner == Some(p1_id) {
        GameOutcome::Player1Win
    } else {
        GameOutcome::Player2Win
    };

    // Rewind back to midpoint
    while game.undo_log.len() > start_undo_size {
        game.undo().expect("Undo should succeed");
    }

    (turns_played, actions_played, outcome)
}

fn main() {
    // Parse batch size from command line (default 1000)
    let batch_size = std::env::args()
        .nth(1)
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(1000);

    println!("=== Rewind + Play Again Benchmark ===");
    println!("Batch size: {} games", batch_size);
    println!("Deck: {}", BASELINE_DECK_PATH);
    println!();

    // Load resources
    println!("Loading deck and card database...");
    let setup = BenchmarkSetup::load(BASELINE_DECK_PATH).expect("Failed to load setup");

    // Create midgame state
    let seed = 42u64;
    println!("Creating midgame state (seed {})...", seed);
    let (midgame_template, original_total_actions) = create_midgame_state(&setup, seed);
    println!("  Full game had {} actions", original_total_actions);
    println!("  Starting from midpoint (undo log cleared)");
    println!();

    // Execute batch
    println!("Executing batch of {} games...", batch_size);
    let mut game = midgame_template.clone();

    let mut batch_turns = 0u32;
    let mut batch_actions = 0usize;
    let mut p1_wins = 0usize;
    let mut p2_wins = 0usize;

    let region = Region::new(GLOBAL);
    let start = Instant::now();

    for i in 0..batch_size {
        let game_seed = seed.wrapping_add(i as u64);
        let (turns_played, actions_played, outcome) = execute_single_game(&mut game, game_seed);

        batch_turns += turns_played;
        batch_actions += actions_played;

        match outcome {
            GameOutcome::Player1Win => p1_wins += 1,
            GameOutcome::Player2Win => p2_wins += 1,
        }
    }

    let duration = start.elapsed();
    let stats = region.change();

    // Print results
    println!();
    println!("=== Results ===");
    println!("Total games: {}", batch_size);
    println!("Total duration: {:.3}s", duration.as_secs_f64());
    println!(
        "Avg duration/game: {:.3}ms",
        duration.as_secs_f64() * 1000.0 / batch_size as f64
    );
    println!();

    println!("=== Game Metrics ===");
    println!("Total turns: {}", batch_turns);
    println!("Total actions: {}", batch_actions);
    println!("Avg turns/game: {:.2}", batch_turns as f64 / batch_size as f64);
    println!("Avg actions/game: {:.2}", batch_actions as f64 / batch_size as f64);
    println!("Actions/turn: {:.2}", batch_actions as f64 / batch_turns as f64);
    println!();

    println!("=== Allocation Metrics ===");
    println!("Total bytes allocated: {}", stats.bytes_allocated);
    println!("Total bytes deallocated: {}", stats.bytes_deallocated);
    println!(
        "Net bytes: {}",
        stats.bytes_allocated as i64 - stats.bytes_deallocated as i64
    );
    println!(
        "Avg bytes/game: {:.2}",
        stats.bytes_allocated as f64 / batch_size as f64
    );
    println!("Bytes/turn: {:.2}", stats.bytes_allocated as f64 / batch_turns as f64);
    println!();

    println!("=== Win Rates ===");
    println!(
        "P1 wins: {} ({:.1}%)",
        p1_wins,
        100.0 * p1_wins as f64 / batch_size as f64
    );
    println!(
        "P2 wins: {} ({:.1}%)",
        p2_wins,
        100.0 * p2_wins as f64 / batch_size as f64
    );
}
