//! Standalone binary for running rewind + play again benchmark
//!
//! This bypasses Criterion and runs a single batch directly, printing metrics.
//!
//! Usage:
//!   cargo run --release --package mtg-benchmarks --bin rewind_bench [batch_size]
//!
//! Default batch size: 1000 games

// Import allocator for stats tracking - we define it here, not in allocator module
use stats_alloc::{Region, StatsAlloc, INSTRUMENTED_SYSTEM};
use std::alloc::System;

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

// Include benchmark allocator types (but not the global allocator)
mod allocator {
    pub use stats_alloc::{Region, INSTRUMENTED_SYSTEM};
    use std::alloc::System;

    /// Allocation statistics - works with or without tracking
    #[derive(Debug, Clone, Copy, Default)]
    #[allow(dead_code)]
    pub struct AllocStats {
        pub bytes_allocated: usize,
        pub bytes_deallocated: usize,
        pub bytes_reallocated: usize,
        pub allocations: usize,
        pub deallocations: usize,
        pub reallocations: usize,
    }

    impl AllocStats {
        pub const fn zero() -> Self {
            AllocStats {
                bytes_allocated: 0,
                bytes_deallocated: 0,
                bytes_reallocated: 0,
                allocations: 0,
                deallocations: 0,
                reallocations: 0,
            }
        }
    }

    impl From<stats_alloc::Stats> for AllocStats {
        fn from(stats: stats_alloc::Stats) -> Self {
            AllocStats {
                bytes_allocated: stats.bytes_allocated,
                bytes_deallocated: stats.bytes_deallocated,
                bytes_reallocated: stats.bytes_reallocated.max(0) as usize,
                allocations: stats.allocations,
                deallocations: stats.deallocations,
                reallocations: stats.reallocations,
            }
        }
    }

    pub struct AllocTracker {
        region: Region<'static, System>,
    }

    impl AllocTracker {
        pub fn new() -> Self {
            AllocTracker {
                region: Region::new(&INSTRUMENTED_SYSTEM),
            }
        }

        pub fn stats(&self) -> AllocStats {
            self.region.change().into()
        }
    }
}

#[path = "../../benches/pinned_thread_pool.rs"]
#[allow(dead_code)]
mod pinned_thread_pool;

// Define dependencies that rewind_play_again needs
use mtg_forge_rs::game::GameState;
use mtg_forge_rs::loader::{
    prefetch_deck_cards, AsyncCardDatabase as CardDatabase, DeckList, DeckLoader, GameInitializer,
};
use std::path::PathBuf;
use tokio::runtime::Runtime;

const BASELINE_DECK_PATH: &str = "decks/old_school/03_robots_jesseisbak.dck";

/// Setup data needed for benchmarking (loaded once, reused across iterations)
struct BenchmarkSetup {
    card_db: CardDatabase,
    deck1: DeckList,
    deck2: DeckList,
    runtime: Runtime,
}

impl BenchmarkSetup {
    fn load_same_deck(deck_path: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let runtime = Runtime::new()?;
        let cardsfolder = PathBuf::from("cardsfolder");
        let card_db = CardDatabase::new(cardsfolder);
        let deck1 = DeckLoader::load_from_file(&PathBuf::from(deck_path))?;
        let deck2 = DeckLoader::load_from_file(&PathBuf::from(deck_path))?;

        runtime.block_on(async {
            prefetch_deck_cards(&card_db, &deck1).await?;
            prefetch_deck_cards(&card_db, &deck2).await
        })?;

        Ok(BenchmarkSetup {
            card_db,
            deck1,
            deck2,
            runtime,
        })
    }
}

/// Helper function to ensure we're in the correct working directory
fn ensure_correct_working_directory() {
    use std::env;

    let mut current_dir = env::current_dir().expect("Failed to get current directory");

    // Check if current directory has 'decks' subdirectory
    if current_dir.join("decks").exists() {
        return; // Already in correct directory
    }

    // Search up the directory tree
    while let Some(parent) = current_dir.parent() {
        if parent.join("decks").exists() {
            env::set_current_dir(parent).expect("Failed to change directory");
            eprintln!("Changed working directory to: {}", parent.display());
            return;
        }
        current_dir = parent.to_path_buf();
    }

    panic!("Could not find workspace root (directory containing 'decks')");
}

/// Create a mid-game state by playing to halfway point
fn create_midgame_state(setup: &BenchmarkSetup, seed: u64) -> (GameState, usize) {
    use mtg_forge_rs::game::{random_controller::RandomController, GameLoop, VerbosityLevel};

    let game_init = GameInitializer::new(&setup.card_db);
    let mut game = setup
        .runtime
        .block_on(async {
            game_init
                .init_game(
                    "Player 1".to_string(),
                    &setup.deck1,
                    "Player 2".to_string(),
                    &setup.deck2,
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
    let rewind_target = total_actions / 2; // Rewind to middle of game

    // Rewind to the target position
    while game.undo_log.len() > rewind_target {
        game.undo().expect("Undo should succeed");
    }

    // Clear the undo log at midpoint - we only care about 50%-100% actions
    game.undo_log.clear();

    (game, total_actions)
}

// Now include the rewind_play_again module which depends on the above
#[path = "../../benches/lib/rewind_play_again.rs"]
mod rewind_play_again;

use rewind_play_again::RewindPlayAgain;

fn main() {
    // Parse batch size from command line (default 1000)
    let batch_size = std::env::args()
        .nth(1)
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(1000);

    println!("=== Rewind + Play Again Benchmark ===");
    println!("Batch size: {} games", batch_size);
    println!();

    // Create benchmark instance (loads deck and creates midgame state)
    println!("Initializing benchmark...");
    let benchmark = RewindPlayAgain::new("SEQUENTIAL");
    let seed = benchmark.seed();
    println!("  Seed: {}", seed);
    println!();

    // Execute batch
    println!("Executing batch of {} games...", batch_size);
    let region = Region::new(GLOBAL);
    let batch_duration = benchmark.execute_batch_sequential(batch_size);
    let stats = region.change();

    // Get aggregated metrics
    let metrics = benchmark.get_aggregated_metrics();
    let total_games = benchmark.get_total_games();

    // Print results
    println!();
    println!("=== Results ===");
    println!("Total games: {}", total_games);
    println!("Total duration: {:.3}s", batch_duration.as_secs_f64());
    println!(
        "Avg duration/game: {:.3}ms",
        batch_duration.as_secs_f64() * 1000.0 / total_games as f64
    );
    println!();

    println!("=== Game Metrics ===");
    println!("Total turns: {}", metrics.turns);
    println!("Total actions: {}", metrics.actions);
    println!("Avg turns/game: {:.2}", metrics.turns as f64 / total_games as f64);
    println!("Avg actions/game: {:.2}", metrics.actions as f64 / total_games as f64);
    println!("Actions/turn: {:.2}", metrics.actions_per_turn());
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
        stats.bytes_allocated as f64 / total_games as f64
    );
    println!("Bytes/turn: {:.2}", metrics.bytes_per_turn());
    println!();

    // Print win rates
    benchmark.print_win_rates("SEQUENTIAL");
}
