//! Utility functions for benchmark infrastructure
//!
//! This module provides helper functions for benchmarks including
//! working directory management, game state creation, and resource loading.

use mtg_forge_rs::game::{random_controller::RandomController, GameLoop, GameState, VerbosityLevel};
use mtg_forge_rs::loader::{
    prefetch_deck_cards, AsyncCardDatabase as CardDatabase, DeckList, DeckLoader, GameInitializer,
};
use mtg_forge_rs::Result;
use std::path::PathBuf;
use std::time::Duration;
use tokio::runtime::Runtime;

/// Benchmark measurement time in seconds (used by all benchmarks)
/// Can be overridden via BENCH_MEASUREMENT_TIME_SECS environment variable
#[allow(dead_code)] // Used by some binaries but not all
const BENCHMARK_TIME_SECS: u64 = 10;

/// Get benchmark measurement time from environment or default
#[allow(dead_code)] // Used by some binaries but not all
pub fn get_benchmark_measurement_time() -> Duration {
    let secs = std::env::var("BENCH_MEASUREMENT_TIME_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(BENCHMARK_TIME_SECS);
    Duration::from_secs(secs)
}

/// Get number of threads to use for parallel benchmarks
pub fn get_benchmark_num_threads() -> usize {
    // Configure thread count: Check BENCH_NUM_THREADS env var, otherwise use physical cores
    let num_physical_cores = num_cpus::get_physical();
    std::env::var("BENCH_NUM_THREADS")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(num_physical_cores)
}


/// Helper function to ensure we're in the correct working directory
///
/// Criterion benchmarks may run from various subdirectories inside target/.
/// This function navigates up the directory tree until it finds a directory
/// containing 'decks', which indicates the workspace root.
pub fn ensure_correct_working_directory() {
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

/// Setup data needed for benchmarking (loaded once, reused across iterations)
pub struct BenchmarkSetup {
    pub card_db: CardDatabase,
    pub deck1: DeckList,
    pub deck2: DeckList,
    pub runtime: Runtime,
}

impl BenchmarkSetup {
    pub fn load(deck1_path: &str, deck2_path: &str) -> Result<Self> {
        let runtime = Runtime::new().expect("Failed to create tokio runtime");

        let cardsfolder = PathBuf::from("cardsfolder");
        let card_db = CardDatabase::new(cardsfolder);

        let deck1 = DeckLoader::load_from_file(&PathBuf::from(deck1_path))?;
        let deck2 = DeckLoader::load_from_file(&PathBuf::from(deck2_path))?;

        // Prefetch deck cards
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

    #[allow(dead_code)] // Used by some binaries but not all
    pub fn load_same_deck(deck_path: &str) -> Result<Self> {
        Self::load(deck_path, deck_path)
    }
}

/// Create a game state at a specific point in the game
///
/// This helper plays a full game, then rewinds to a specified percentage point.
/// The undo log is cleared at this point.
///
/// # Parameters
/// - `setup`: Benchmark setup with card database and decks
/// - `seed`: Initial RNG seed for the game
/// - `rewind_percent`: Percentage of actions to keep (0.0 = start, 1.0 = end)
///
/// # Returns
/// A tuple of (initial GameState before any play, GameState at rewind point, original total actions)
pub fn create_game_at_percent(setup: &BenchmarkSetup, seed: u64, rewind_percent: f64) -> (GameState, GameState, usize) {
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

    // Clone the initial state before playing
    let initial_game = game.clone();

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
    let rewind_target = (total_actions as f64 * rewind_percent) as usize;

    // Rewind to the target position
    while game.undo_log.len() > rewind_target {
        game.undo().expect("Undo should succeed");
    }

    // Clear the undo log at rewind point
    game.undo_log.clear();

    (initial_game, game, total_actions)
}

/// Create a mid-game state by playing to halfway point
///
/// This helper creates a game state at the 50% mark (halfway through a full game).
/// The undo log is cleared at this point, so we only track actions from 50%-100%.
///
/// # Parameters
/// - `setup`: Benchmark setup with card database and decks
/// - `seed`: Initial RNG seed for the first half of the game
///
/// # Returns
/// A tuple of (GameState at midpoint, original total actions before clearing undo log)
#[allow(dead_code)] // Kept for backward compatibility, use create_game_at_percent instead
pub fn create_midgame_state(setup: &BenchmarkSetup, seed: u64) -> (GameState, usize) {
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

/// Helper function to print aggregated metrics
///
/// Note: The "Avg duration/game" shown here is a naive average (total_time / iterations).
/// For accurate per-iteration timing, refer to Criterion's statistical estimate shown above,
/// which accounts for outliers, warmup effects, and provides confidence intervals.
#[allow(dead_code)] // Used by some binaries but not all
pub fn print_aggregated_metrics(mode: &str, seed: u64, aggregated: &super::types::GameMetrics, iteration_count: usize) {
    eprintln!("\n=== Aggregated Metrics - {mode} Mode (seed {seed}, {iteration_count} games) ===");
    eprintln!("  Total turns: {}", aggregated.turns);
    eprintln!("  Total actions: {}", aggregated.actions);
    eprintln!("  Total duration: {:?}", aggregated.duration);
    eprintln!(
        "  Avg turns/game: {:.2}",
        aggregated.turns as f64 / iteration_count as f64
    );
    eprintln!(
        "  Avg actions/game: {:.2}",
        aggregated.actions as f64 / iteration_count as f64
    );
    eprintln!(
        "  Avg duration/game (naive): {:.2?}",
        aggregated.duration / iteration_count as u32
    );
    eprintln!("  Games/sec: {:.2}", aggregated.avg_games_per_sec(iteration_count));
    eprintln!("  Actions/sec: {:.2}", aggregated.actions_per_sec());
    eprintln!("  Turns/sec: {:.2}", aggregated.turns_per_sec());
    eprintln!("  Actions/turn: {:.2}", aggregated.actions_per_turn());
    eprintln!("  Total bytes allocated: {}", aggregated.bytes_allocated);
    eprintln!("  Total bytes deallocated: {}", aggregated.bytes_deallocated);
    eprintln!("  Net bytes: {}", aggregated.net_bytes_allocated());
    eprintln!(
        "  Avg bytes/game: {:.2}",
        aggregated.bytes_allocated as f64 / iteration_count as f64
    );
    eprintln!("  Bytes/turn: {:.2}", aggregated.bytes_per_turn());
    eprintln!("  Bytes/sec: {:.2}", aggregated.bytes_per_sec());
    eprintln!("\nNote: For authoritative per-iteration timing, see Criterion's estimate above.");
}
