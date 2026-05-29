//! DHAT heap profiling benchmark
//!
//! This benchmark runs a representative game workload under dhat profiling
//! to identify allocation hotspots with full Rust symbol information.
//!
//! Unlike heaptrack (which lacks Rust symbols), dhat-rs provides:
//! - Full function names and source locations
//! - Per-call-site allocation breakdowns
//! - Interactive visualization via dh_view.html
//!
//! Usage:
//!   make dhatprofile                         # Recommended: runs profiling + analysis
//!   cargo bench --bench dhat_profile         # Direct: just run profiling
//!   python3 scripts/analyze_dhat.py          # Analyze existing results
//!
//! Output: dhat-heap.json (view at https://nnethercote.github.io/dh_view/dh_view.html)
//!
//! This benchmark uses the "rewind + play again" pattern to isolate forward gameplay
//! allocations, excluding one-time initialization overhead. See benchmarks in
//! game_benchmark.rs for comparison with other profiling approaches.

use mtg_engine::{
    game::{random_controller::RandomController, GameLoop, VerbosityLevel},
    loader::{prefetch_deck_cards, AsyncCardDatabase as CardDatabase, DeckLoader, GameInitializer},
};
use std::path::PathBuf;
use tokio::runtime::Runtime;

#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

fn main() {
    // Start DHAT profiling
    let _profiler = dhat::Profiler::new_heap();

    let runtime = Runtime::new().expect("Failed to create tokio runtime");

    // Load deck and card database
    let cardsfolder = PathBuf::from("../cardsfolder");
    let card_db = CardDatabase::new(cardsfolder);

    let deck_path = "../decks/simple_bolt.dck";
    let deck = DeckLoader::load_from_file(&PathBuf::from(deck_path)).expect("Failed to load deck");

    // Prefetch cards
    runtime
        .block_on(async { prefetch_deck_cards(&card_db, &deck).await })
        .expect("Failed to prefetch cards");

    // Create initial game state
    let game_init = GameInitializer::new(&card_db);
    let mut game = runtime
        .block_on(async {
            game_init
                .init_game("Player 1".to_string(), &deck, "Player 2".to_string(), &deck, 20)
                .await
        })
        .expect("Failed to initialize game");

    let seed = 42u64;
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

        let mut controller1 = RandomController::with_seed(p1_id, 42);
        let mut controller2 = RandomController::with_seed(p2_id, 42);

        let mut game_loop = GameLoop::new(&mut game).with_verbosity(VerbosityLevel::Silent);
        let _ = game_loop
            .run_game(&mut controller1, &mut controller2)
            .expect("Initial game should complete");
    }

    let actions_count = game.undo_log.len();
    let rewind_target = actions_count / 2; // Rewind to middle of game

    eprintln!("DHAT Profiling: Rewind + Play Again pattern");
    eprintln!("  Game completed with {} actions in undo log", actions_count);
    eprintln!("  Will profile {} replays from middle of game", 100);

    // Profile 100 iterations of rewind + replay (forward gameplay only)
    for iteration in 0..100 {
        // Rewind to middle (not profiled - we want forward gameplay only)
        let current_actions = game.undo_log.len();
        let rewinds_needed = current_actions - rewind_target;
        for _ in 0..rewinds_needed {
            game.undo().expect("Undo should succeed");
        }

        // Use different seed for each iteration to explore different paths
        let iteration_seed = seed.wrapping_add(iteration as u64);
        game.seed_rng(iteration_seed);

        // Forward gameplay (THIS is what we profile)
        let (p1_id, p2_id) = {
            let mut players_iter = game.players.iter().map(|p| p.id);
            (
                players_iter.next().expect("Should have player 1"),
                players_iter.next().expect("Should have player 2"),
            )
        };

        let mut controller1 = RandomController::with_seed(p1_id, iteration_seed);
        let mut controller2 = RandomController::with_seed(p2_id, iteration_seed);

        let mut game_loop = GameLoop::new(&mut game).with_verbosity(VerbosityLevel::Silent);
        let _ = game_loop
            .run_game(&mut controller1, &mut controller2)
            .expect("Game should complete");
    }

    eprintln!("Profiling complete! Output: dhat-heap.json");
    eprintln!("View with: https://nnethercote.github.io/dh_view/dh_view.html");

    // Profiler drops here, writing dhat-heap.json
}
