//! Performance benchmarks for MTG Forge game engine
//!
//! This benchmark measures game execution performance using Criterion.rs.
//! It supports three different iteration modes:
//!
//! 1. **Fresh** - Allocate a new game for each iteration
//! 2. **Rewind** - Use undo log to rewind game to start (NOT YET IMPLEMENTED)
//! 3. **Snapshot** - Save/restore game state each iteration (NOT YET IMPLEMENTED)
//!
//! The benchmark is based on RandomController vs RandomController playing
//! with simple_bolt.dck (Mountains + Lightning Bolts).
//!
//! ##[path = "rewind_play_again_module.rs"]Allocator Configuration
//!
//! By default, allocation tracking is ENABLED using stats_alloc.
//! Use feature flags to select different allocators:
//! - `bench-stats-alloc`: stats_alloc with allocation tracking (DEFAULT)
//! - `bench-mimalloc`: mimalloc for maximum performance (no tracking)
//! - `bench-jemalloc`: jemalloc for performance (no tracking)
//!
//! Run with: `cargo bench` for tracking (default)
//! Run with: `cargo bench --no-default-features --features bench-mimalloc` for max performance
//!
//! ##[path = "rewind_play_again_module.rs"]Lazy Initialization Pattern
//!
//! **IMPORTANT**: All benchmarks MUST use lazy initialization to avoid running setup code
//! during `cargo bench --list` (which only lists benchmarks without running them).
//!
//! ###[path = "rewind_play_again_module.rs"]Problem
//!
//! Criterion benchmark functions are executed at registration time to gather metadata,
//! not just at execution time. If you create expensive state (like game instances) before
//! calling `bench_function()`, that initialization will run even when listing benchmarks:
//!
//! ```rust,ignore
//! // BAD: This runs during --list!
//! let game = create_midgame_state(&setup, seed);
//! group.bench_function("my_benchmark", |b| {
//!     b.iter(|| { /* use game */ });
//! });
//! ```
//!
//! ###[path = "rewind_play_again_module.rs"]Solution
//!
//! Use `Option<T>` with lazy initialization inside the benchmark closure:
//!
//! ```rust,ignore
//! // GOOD: This only runs during actual benchmarking
//! let mut game_template: Option<GameState> = None;
//!
//! group.bench_function("my_benchmark", |b| {
//!     b.iter(|| {
//!         // Initialize on first iteration
//!         if game_template.is_none() {
//!             eprintln!("Initializing benchmark state...");
//!             let game = create_midgame_state(&setup, seed);
//!             game_template = Some(game);
//!         }
//!
//!         let game = game_template.as_ref().unwrap();
//!         // Use game...
//!     });
//! });
//! ```
//!
//! ###[path = "rewind_play_again_module.rs"]Examples in This File
//!
//! - `bench_game_snapshot` (line ~815): Uses `Option<GameState>` for lazy game initialization
//! - `bench_game_rewind` (line ~890): Uses `Option<GameState>` for lazy game initialization
//! - `bench_game_rewind_play_again` (line ~1038): Uses `Option<GameState>` for midgame state
//! - `bench_game_pinned_par_rewind_play_again` (line ~1134): Uses `Option<GameState>` inside `iter_custom`
//! - `bench_game_par_rewind_play_again` (line ~1290): Uses `Option<GameState>` inside `iter_custom`
//! - `bench_save_snapshot` (line ~1442): Uses `Option<GameSnapshot>` for snapshot state
//!
//! ###[path = "rewind_play_again_module.rs"]Verification
//!
//! To verify benchmarks use lazy initialization, run:
//!
//! ```bash
//! cargo bench --bench game_benchmark -- --list
//! ```
//!
//! This should complete instantly without printing initialization messages. If you see
//! "Initializing..." or game state creation messages, the benchmark needs fixing.

mod allocator;
mod pinned_thread_pool;

#[path = "lib/mod.rs"]
mod benchlib;

use allocator::{AllocStats, AllocTracker};
use benchlib::{
    ensure_correct_working_directory, get_benchmark_measurement_time, print_aggregated_metrics, BatchBenchmark,
    BenchmarkSetup, GameMetrics, ParPinned, ParRayon, RewindPlayAgain, RewindPlayAgainConfig, BASELINE_DECK_PATH,
};
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use mtg_forge_rs::{
    game::{random_controller::RandomController, GameLoop, GameSnapshot, GameState, VerbosityLevel},
    loader::GameInitializer,
    Result,
};
use std::path::PathBuf;
use std::time::Duration;
use tempfile::tempdir;

/// Helper macro to create allocation tracker
macro_rules! stats_region {
    () => {{
        Some(AllocTracker::new())
    }};
}

/// Helper macro to get stats from tracker
macro_rules! get_stats {
    ($tracker:expr) => {{
        if let Some(ref t) = $tracker {
            t.stats()
        } else {
            AllocStats::zero()
        }
    }};
}

/// Run a single game and collect metrics
/// Takes a game initializer function to support different initialization strategies
fn run_game_with_metrics<F>(seed: u64, game_init_fn: F) -> Result<GameMetrics>
where
    F: FnOnce() -> Result<mtg_forge_rs::game::GameState>,
{
    let reg = stats_region!();
    let start = std::time::Instant::now();

    // Initialize game using provided function
    let mut game = game_init_fn()?;
    game.seed_rng(seed);

    // Create random controllers
    let (p1_id, p2_id) = {
        let mut players_iter = game.players.iter().map(|p| p.id);
        (
            players_iter.next().expect("Should have player 1"),
            players_iter.next().expect("Should have player 2"),
        )
    };

    let mut controller1 = RandomController::with_seed(p1_id, 42);
    let mut controller2 = RandomController::with_seed(p2_id, 42);

    // Run game (still within timing)
    let mut game_loop = GameLoop::new(&mut game).with_verbosity(VerbosityLevel::Silent);
    let result = game_loop.run_game(&mut controller1, &mut controller2)?;

    let duration = start.elapsed();

    // Collect metrics
    let actions = game_loop.game.undo_log.len();
    let stats = get_stats!(reg);

    let metrics = GameMetrics {
        turns: result.turns_played,
        actions,
        duration,
        bytes_allocated: stats.bytes_allocated,
        bytes_deallocated: stats.bytes_deallocated,
    };

    Ok(metrics)
}

/// Run a single game with in-memory logging enabled at Normal verbosity
fn run_game_with_logging<F>(seed: u64, game_init_fn: F) -> Result<GameMetrics>
where
    F: FnOnce() -> Result<mtg_forge_rs::game::GameState>,
{
    use std::fs::OpenOptions;
    use std::os::fd::AsRawFd;

    let reg = stats_region!();
    let start = std::time::Instant::now();

    // Initialize game using provided function
    let mut game = game_init_fn()?;
    game.seed_rng(seed);

    // Enable log capture
    game.logger.enable_capture();

    // Redirect stdout to /dev/null to avoid benchmark noise
    // (Logger may still write to stdout even with capture enabled)
    let devnull = OpenOptions::new()
        .write(true)
        .open("/dev/null")
        .expect("Failed to open /dev/null");
    let orig_stdout = unsafe { libc::dup(libc::STDOUT_FILENO) };
    unsafe {
        libc::dup2(devnull.as_raw_fd(), libc::STDOUT_FILENO);
    }

    // Create random controllers
    let (p1_id, p2_id) = {
        let mut players_iter = game.players.iter().map(|p| p.id);
        (
            players_iter.next().expect("Should have player 1"),
            players_iter.next().expect("Should have player 2"),
        )
    };

    let mut controller1 = RandomController::with_seed(p1_id, 42);
    let mut controller2 = RandomController::with_seed(p2_id, 42);

    // Run game with Normal verbosity to capture logs
    let mut game_loop = GameLoop::new(&mut game).with_verbosity(VerbosityLevel::Normal);
    let result = game_loop.run_game(&mut controller1, &mut controller2)?;

    // Restore stdout
    unsafe {
        libc::dup2(orig_stdout, libc::STDOUT_FILENO);
        libc::close(orig_stdout);
    }

    let duration = start.elapsed();

    // Collect metrics
    let actions = game_loop.game.undo_log.len();
    let stats = get_stats!(reg);

    let metrics = GameMetrics {
        turns: result.turns_played,
        actions,
        duration,
        bytes_allocated: stats.bytes_allocated,
        bytes_deallocated: stats.bytes_deallocated,
    };

    // Note: We don't report log entries per iteration to avoid spam during benchmarking
    // Log verification happens in tests, not benchmarks

    Ok(metrics)
}

/// Run a single game with stdout logging at Normal verbosity (not capturing)
/// This tests the reusable buffer optimization
fn run_game_with_stdout_logging<F>(seed: u64, game_init_fn: F) -> Result<GameMetrics>
where
    F: FnOnce() -> Result<mtg_forge_rs::game::GameState>,
{
    use std::fs::OpenOptions;
    use std::os::fd::AsRawFd;

    let reg = stats_region!();
    let start = std::time::Instant::now();

    // Initialize game using provided function
    let mut game = game_init_fn()?;
    game.seed_rng(seed);

    // DO NOT enable log capture - we want stdout logging

    // Redirect stdout to /dev/null to avoid benchmark noise
    let devnull = OpenOptions::new()
        .write(true)
        .open("/dev/null")
        .expect("Failed to open /dev/null");
    let orig_stdout = unsafe { libc::dup(libc::STDOUT_FILENO) };
    unsafe {
        libc::dup2(devnull.as_raw_fd(), libc::STDOUT_FILENO);
    }

    // Create random controllers
    let (p1_id, p2_id) = {
        let mut players_iter = game.players.iter().map(|p| p.id);
        (
            players_iter.next().expect("Should have player 1"),
            players_iter.next().expect("Should have player 2"),
        )
    };

    let mut controller1 = RandomController::with_seed(p1_id, 42);
    let mut controller2 = RandomController::with_seed(p2_id, 42);

    // Run game with Normal verbosity (logs to stdout via reusable buffer)
    let mut game_loop = GameLoop::new(&mut game).with_verbosity(VerbosityLevel::Normal);
    let result = game_loop.run_game(&mut controller1, &mut controller2)?;

    // Restore stdout
    unsafe {
        libc::dup2(orig_stdout, libc::STDOUT_FILENO);
        libc::close(orig_stdout);
    }

    let duration = start.elapsed();

    // Collect metrics
    let actions = game_loop.game.undo_log.len();
    let stats = get_stats!(reg);

    let metrics = GameMetrics {
        turns: result.turns_played,
        actions,
        duration,
        bytes_allocated: stats.bytes_allocated,
        bytes_deallocated: stats.bytes_deallocated,
    };

    Ok(metrics)
}

/// Benchmark: Fresh mode - allocate new game each iteration
fn bench_game_fresh(c: &mut Criterion) {
    ensure_correct_working_directory();

    // Check if test resources exist and load once
    let setup = match BenchmarkSetup::load_same_deck(BASELINE_DECK_PATH) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Skipping benchmark - failed to load resources: {e}");
            return;
        }
    };

    let mut group = c.benchmark_group("game_execution");

    // Configure for long-running benchmarks
    group.sample_size(10); // Reduce sample size since games can be long
    group.measurement_time(get_benchmark_measurement_time());

    let seed = 42u64;

    // Accumulator for aggregating metrics across benchmark iterations
    let mut aggregated = GameMetrics {
        turns: 0,
        actions: 0,
        duration: Duration::ZERO,
        bytes_allocated: 0,
        bytes_deallocated: 0,
    };
    let mut iteration_count = 0;

    group.bench_function("fresh", |b| {
        b.iter(|| {
            let game_init_fn = || {
                let game_init = GameInitializer::new(&setup.card_db);
                setup.runtime.block_on(async {
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
            };

            let metrics =
                run_game_with_metrics(black_box(seed), game_init_fn).expect("Game should complete successfully");
            aggregated += metrics.clone();
            iteration_count += 1;
        });
    });

    if iteration_count > 0 {
        print_aggregated_metrics("Fresh", seed, &aggregated, iteration_count);
    }

    group.finish();
}

/// Benchmark: Fresh mode with in-memory logging at Normal verbosity
/// Measures allocation overhead of logging infrastructure
fn bench_game_fresh_with_logging(c: &mut Criterion) {
    ensure_correct_working_directory();

    // Check if test resources exist and load once
    let setup = match BenchmarkSetup::load_same_deck(BASELINE_DECK_PATH) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Skipping benchmark - failed to load resources: {e}");
            return;
        }
    };

    let mut group = c.benchmark_group("game_execution");

    // Configure for long-running benchmarks
    group.sample_size(10);
    group.measurement_time(get_benchmark_measurement_time());

    let seed = 42u64;

    // Accumulator for aggregating metrics across benchmark iterations
    let mut aggregated = GameMetrics {
        turns: 0,
        actions: 0,
        duration: Duration::ZERO,
        bytes_allocated: 0,
        bytes_deallocated: 0,
    };
    let mut iteration_count = 0;

    group.bench_function("fresh_logging", |b| {
        b.iter(|| {
            let game_init_fn = || {
                let game_init = GameInitializer::new(&setup.card_db);
                setup.runtime.block_on(async {
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
            };

            let metrics =
                run_game_with_logging(black_box(seed), game_init_fn).expect("Game should complete successfully");
            aggregated += metrics.clone();
            iteration_count += 1;
        });
    });

    if iteration_count > 0 {
        print_aggregated_metrics("Fresh with Logging", seed, &aggregated, iteration_count);
    }

    group.finish();
}

/// Benchmark: Fresh mode with stdout logging at Normal verbosity (redirected to /dev/null)
/// Measures allocation overhead with reusable buffer optimization
fn bench_game_fresh_with_stdout_logging(c: &mut Criterion) {
    ensure_correct_working_directory();

    // Check if test resources exist and load once
    let setup = match BenchmarkSetup::load_same_deck(BASELINE_DECK_PATH) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Skipping benchmark - failed to load resources: {e}");
            return;
        }
    };

    let mut group = c.benchmark_group("game_execution");

    // Configure for long-running benchmarks
    group.sample_size(10);
    group.measurement_time(get_benchmark_measurement_time());

    let seed = 42u64;

    // Accumulator for aggregating metrics across benchmark iterations
    let mut aggregated = GameMetrics {
        turns: 0,
        actions: 0,
        duration: Duration::ZERO,
        bytes_allocated: 0,
        bytes_deallocated: 0,
    };
    let mut iteration_count = 0;

    group.bench_function("fresh_stdout_logging", |b| {
        b.iter(|| {
            let game_init_fn = || {
                let game_init = GameInitializer::new(&setup.card_db);
                setup.runtime.block_on(async {
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
            };

            let metrics =
                run_game_with_stdout_logging(black_box(seed), game_init_fn).expect("Game should complete successfully");
            aggregated += metrics.clone();
            iteration_count += 1;
        });
    });

    if iteration_count > 0 {
        eprintln!(
            "\n=== Aggregated Metrics - Fresh with Stdout Logging Mode (seed {seed}, {iteration_count} games) ==="
        );
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
            "  Avg duration/game: {:.2?}",
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
    }

    group.finish();
}

/// Benchmark: Snapshot mode - save/restore game state each iteration
/// Uses Clone to create a fresh copy of the initial game state
fn bench_game_snapshot(c: &mut Criterion) {
    ensure_correct_working_directory();

    // Check if test resources exist and load once
    let setup = match BenchmarkSetup::load_same_deck(BASELINE_DECK_PATH) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Skipping benchmark - failed to load resources: {e}");
            return;
        }
    };

    let mut group = c.benchmark_group("game_execution");
    group.sample_size(10);
    group.measurement_time(get_benchmark_measurement_time());

    let seed = 42u64;

    // Accumulator for aggregating metrics across benchmark iterations
    let mut aggregated = GameMetrics {
        turns: 0,
        actions: 0,
        duration: Duration::ZERO,
        bytes_allocated: 0,
        bytes_deallocated: 0,
    };
    let mut iteration_count = 0;

    // Lazy initialization - only create initial game on first iteration
    let mut initial_game = None;

    group.bench_function("snapshot", |b| {
        b.iter(|| {
            // Initialize on first iteration
            if initial_game.is_none() {
                eprintln!("\nSnapshot mode (seed {seed}):");
                eprintln!("  Pre-creating initial game state for cloning...");

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
                initial_game = Some(game);
            }

            let game_template = initial_game.as_ref().unwrap();
            let game_init_fn = || Ok(game_template.clone());
            let metrics = run_game_with_metrics(seed, game_init_fn).expect("Game should complete successfully");
            aggregated += metrics.clone();
            iteration_count += 1;
        });
    });

    if iteration_count > 0 {
        print_aggregated_metrics("Snapshot", seed, &aggregated, iteration_count);
    }

    group.finish();
}

/// Benchmark: Rewind mode - use undo log to rewind game
/// Measures the cost of rewinding using undo() for tree search
fn bench_game_rewind(c: &mut Criterion) {
    ensure_correct_working_directory();

    // Check if test resources exist and load once
    let setup = match BenchmarkSetup::load_same_deck(BASELINE_DECK_PATH) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Skipping benchmark - failed to load resources: {e}");
            return;
        }
    };

    let mut group = c.benchmark_group("game_execution");
    group.sample_size(10);
    group.measurement_time(get_benchmark_measurement_time());

    let seed = 42u64;

    // Accumulator for aggregating metrics
    let mut aggregated = GameMetrics {
        turns: 0,
        actions: 0,
        duration: Duration::ZERO,
        bytes_allocated: 0,
        bytes_deallocated: 0,
    };
    let mut iteration_count = 0;

    // Lazy initialization - only create and run initial game on first iteration
    let mut initial_game: Option<GameState> = None;

    group.bench_function("rewind", |b| {
        b.iter(|| {
            // Initialize on first iteration
            if initial_game.is_none() {
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

                    let mut controller1 = RandomController::with_seed(p1_id, 42);
                    let mut controller2 = RandomController::with_seed(p2_id, 42);

                    let mut game_loop = GameLoop::new(&mut game).with_verbosity(VerbosityLevel::Silent);
                    let _ = game_loop
                        .run_game(&mut controller1, &mut controller2)
                        .expect("Initial game should complete");
                }

                let actions_count = game.undo_log.len();
                eprintln!("\nRewind mode (seed {seed}):");
                eprintln!("  Game completed with {} actions in undo log", actions_count);
                eprintln!("  Will rewind to start for each iteration...");

                initial_game = Some(game);
            }

            let game = initial_game.as_mut().unwrap();

            let reg = stats_region!();
            let start = std::time::Instant::now();

            // Rewind all actions to get back to initial state
            let mut rewind_count = 0;
            while game.undo().expect("Undo should succeed").is_some() {
                rewind_count += 1;
            }

            let duration = start.elapsed();
            let stats = get_stats!(reg);

            // Record metrics for the rewind operation
            let metrics = GameMetrics {
                turns: 18, // We know from the fresh run this is 18 turns
                actions: rewind_count,
                duration,
                bytes_allocated: stats.bytes_allocated,
                bytes_deallocated: stats.bytes_deallocated,
            };

            aggregated += metrics;
            iteration_count += 1;

            // Re-run the game to populate undo log for next iteration
            // (This happens outside the timing, as we're measuring rewind cost)
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

                let mut game_loop = GameLoop::new(game).with_verbosity(VerbosityLevel::Silent);
                let _ = game_loop
                    .run_game(&mut controller1, &mut controller2)
                    .expect("Game should complete");
            }
        });
    });

    if iteration_count > 0 {
        print_aggregated_metrics("Rewind", seed, &aggregated, iteration_count);
    }

    group.finish();
}

/// Benchmark: Rewind + replay with different paths (SEQUENTIAL)
/// Measures complete forward+rewind cycle exploring different game paths
///
/// This benchmark correctly measures the full MCTS simulation workload:
/// 1. Creates midgame state at 50% mark (done once in setup)
/// 2. For each batch of iterations:
///    - Play forward from midpoint to end
///    - Rewind back to midpoint
///    - Repeat with different random seed
/// 3. Times the ENTIRE batch (forward + rewind for all iterations)
/// 4. Tracks win rates for P1 vs P2
///
/// FIXED: Now uses iter_custom to time batches correctly, and reuses a single
/// game instance (no cloning per iteration, just rewind back to start).
fn bench_game_rewind_play_again(c: &mut Criterion) {
    let mut benchmark: Option<RewindPlayAgain> = None;

    let mut group = c.benchmark_group("game_execution");
    group.sample_size(10);
    group.measurement_time(get_benchmark_measurement_time());

    group.bench_function("rewind_play_again", |b| {
        b.iter_custom(|iters| {
            if benchmark.is_none() {
                let config = RewindPlayAgainConfig::default();
                let new_benchmark = RewindPlayAgain::new(config, "SEQUENTIAL");
                benchmark = Some(new_benchmark);
            }

            let bench = benchmark.as_ref().unwrap();
            bench.execute_batch_sequential(iters as usize)
        });
    });

    if let Some(ref bench) = benchmark {
        let total_games = bench.get_total_games();
        if total_games > 0 {
            let aggregated_metrics = bench.get_aggregated_metrics();
            print_aggregated_metrics(
                "Rewind + Play Again (Sequential)",
                bench.seed(),
                &aggregated_metrics,
                total_games,
            );
            bench.print_win_rates("Rewind + Play Again (Sequential)");
        }
    }

    group.finish();
}

/// Benchmark: Parallel rewind + replay with different paths (PARALLEL with Rayon)
/// Measures complete forward+rewind cycle with parallel execution using the RewindPlayAgain module
///
/// This benchmark measures MCTS-style parallel simulation:
/// 1. Creates midgame state at 50% mark (done once in setup)
/// 2. For each batch of iterations:
///    - Clone game states for N threads (outside timing)
///    - Each thread plays forward from midpoint and rewinds back
///    - Times only the actual parallel gameplay
/// 3. Tracks win rates and allocations across all threads
fn bench_game_par_rewind_play_again(c: &mut Criterion) {
    let mut benchmark: Option<ParRayon<RewindPlayAgain>> = None;
    let num_threads = num_cpus::get();

    let mut group = c.benchmark_group("game_execution");
    group.sample_size(10);
    group.measurement_time(get_benchmark_measurement_time());

    group.bench_function("par_rewind_play_again", |b| {
        b.iter_custom(|iters| {
            if benchmark.is_none() {
                let config = RewindPlayAgainConfig::default();
                let new_benchmark = RewindPlayAgain::new(config, "PARALLEL");
                let par_bench = ParRayon::new(new_benchmark);
                benchmark = Some(par_bench);
            }

            let bench = benchmark.as_ref().unwrap();
            bench.execute_batch(iters as usize, num_threads).unwrap()
        });
    });

    if let Some(ref bench) = benchmark {
        let total_games = bench.total_games();
        if total_games > 0 {
            let aggregated_metrics = bench.get_metrics();
            // Access the inner RewindPlayAgain for seed and win rates
            let inner_bench = bench.inner();
            print_aggregated_metrics(
                "Rewind + Play Again (Parallel)",
                inner_bench.seed(),
                &aggregated_metrics,
                total_games,
            );
            inner_bench.print_win_rates("Rewind + Play Again (Parallel)");
        }
    }

    group.finish();
}

/// Benchmark: Parallel rewind + replay with PINNED threads
/// Measures complete forward+rewind cycle with pinned thread execution using the RewindPlayAgain module
///
/// This benchmark measures MCTS-style parallel simulation with pinned threads:
/// 1. Creates midgame state at 50% mark (done once in setup)
/// 2. For each batch of iterations:
///    - Clone game states for N threads (outside timing)
///    - Each thread plays forward from midpoint and rewinds back
///    - Times only the actual parallel gameplay with microsecond-accurate timing
/// 3. Tracks win rates and allocations across all threads
///
/// CRITICAL: Uses custom thread pool with spin barriers for microsecond-accurate timing.
fn bench_game_pinned_par_rewind_play_again(c: &mut Criterion) {
    // Configure thread count: Check BENCH_NUM_THREADS env var, otherwise use physical cores
    let num_physical_cores = num_cpus::get_physical();
    let num_threads = std::env::var("BENCH_NUM_THREADS")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(num_physical_cores);

    let mut benchmark: Option<ParPinned> = None;

    let mut group = c.benchmark_group("game_execution");
    group.sample_size(10);
    group.measurement_time(get_benchmark_measurement_time());

    group.bench_function("pinned_par_rewind_play_again", |b| {
        b.iter_custom(|iters| {
            if benchmark.is_none() {
                let config = RewindPlayAgainConfig::default();
                let new_benchmark = RewindPlayAgain::new(config, "PINNED-PARALLEL");
                let par_bench = ParPinned::new(new_benchmark);
                benchmark = Some(par_bench);
            }

            let bench = benchmark.as_ref().unwrap();
            bench.execute_batch(iters as usize, num_threads).unwrap()
        });
    });

    if let Some(ref bench) = benchmark {
        let total_games = bench.total_games();
        if total_games > 0 {
            let aggregated_metrics = bench.get_metrics();
            // Access the inner RewindPlayAgain for seed and win rates
            let inner_bench = bench.inner();
            print_aggregated_metrics(
                "Rewind + Play Again (Pinned-Parallel)",
                inner_bench.seed(),
                &aggregated_metrics,
                total_games,
            );
            inner_bench.print_win_rates("Rewind + Play Again (Pinned-Parallel)");
        }
    }

    group.finish();
}

/// Benchmark: Save snapshot to file
fn bench_save_snapshot(c: &mut Criterion) {
    ensure_correct_working_directory();

    // Check if test resources exist and load once
    let setup = match BenchmarkSetup::load_same_deck(BASELINE_DECK_PATH) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Skipping benchmark - failed to load resources: {e}");
            return;
        }
    };

    let mut group = c.benchmark_group("snapshot_serialization");
    group.sample_size(10);
    group.measurement_time(get_benchmark_measurement_time());

    let seed = 42u64;

    // Lazy initialization - only create game state and snapshot on first iteration
    let mut snapshot_template: Option<GameSnapshot> = None;
    let mut temp_dir_holder: Option<tempfile::TempDir> = None;
    let mut snapshot_path_holder: Option<PathBuf> = None;

    group.bench_function("save_to_file", |b| {
        b.iter(|| {
            // Initialize on first iteration
            if snapshot_template.is_none() {
                eprintln!("\nSave Snapshot mode (seed {seed}):");
                eprintln!("  Creating game state by running 10 turns...");

                // Create a representative game state by running a game for a few turns
                let mut game = {
                    let game_init = GameInitializer::new(&setup.card_db);
                    setup
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
                        .expect("Failed to initialize game")
                };
                game.seed_rng(seed);

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
                game_loop
                    .run_turns(&mut controller1, &mut controller2, 10)
                    .expect("Game should complete successfully");

                let snapshot = GameSnapshot::new(game.clone(), game.turn.turn_number, vec![]);

                let temp_dir = tempdir().expect("Failed to create temp dir");
                let snapshot_path = temp_dir.path().join("benchmark.snapshot");

                eprintln!("  Snapshot created at turn {}", game.turn.turn_number);

                snapshot_template = Some(snapshot);
                snapshot_path_holder = Some(snapshot_path);
                temp_dir_holder = Some(temp_dir);
            }

            let snapshot = snapshot_template.as_ref().unwrap();
            let snapshot_path = snapshot_path_holder.as_ref().unwrap();

            snapshot
                .save_to_file(
                    black_box(snapshot_path),
                    mtg_forge_rs::game::snapshot::SnapshotFormat::Json,
                )
                .expect("Failed to save snapshot");
        });
    });

    group.finish();
}

/// Benchmark: Old School deck matchup - Mono Black vs The Deck
fn bench_game_old_school_mono_black_vs_the_deck(c: &mut Criterion) {
    ensure_correct_working_directory();

    let setup = match BenchmarkSetup::load(
        "decks/old_school/05_mono_black_rogerbrand.dck",
        "decks/old_school/02_thedeck_peterschnidrig.dck",
    ) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Skipping benchmark - failed to load resources: {e}");
            return;
        }
    };

    let mut group = c.benchmark_group("old_school_matchups");
    group.sample_size(10);
    group.measurement_time(get_benchmark_measurement_time());

    let seed = 42u64;
    let mut aggregated = GameMetrics {
        turns: 0,
        actions: 0,
        duration: Duration::ZERO,
        bytes_allocated: 0,
        bytes_deallocated: 0,
    };
    let mut iteration_count = 0;

    group.bench_function("mono_black_vs_the_deck", |b| {
        b.iter(|| {
            let game_init_fn = || {
                let game_init = GameInitializer::new(&setup.card_db);
                setup.runtime.block_on(async {
                    game_init
                        .init_game(
                            "Mono Black".to_string(),
                            &setup.deck1,
                            "The Deck".to_string(),
                            &setup.deck2,
                            20,
                        )
                        .await
                })
            };

            let metrics =
                run_game_with_metrics(black_box(seed), game_init_fn).expect("Game should complete successfully");
            aggregated += metrics.clone();
            iteration_count += 1;
        });
    });

    if iteration_count > 0 {
        print_aggregated_metrics("Old School: Mono Black vs The Deck", seed, &aggregated, iteration_count);
    }

    group.finish();
}

/// Benchmark: Old School deck matchup - White Weenie mirror
fn bench_game_old_school_white_weenie_mirror(c: &mut Criterion) {
    ensure_correct_working_directory();

    let setup = match BenchmarkSetup::load_same_deck("decks/old_school2/white_weenie_classic.dck") {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Skipping benchmark - failed to load resources: {e}");
            return;
        }
    };

    let mut group = c.benchmark_group("old_school_matchups");
    group.sample_size(10);
    group.measurement_time(get_benchmark_measurement_time());

    let seed = 42u64;
    let mut aggregated = GameMetrics {
        turns: 0,
        actions: 0,
        duration: Duration::ZERO,
        bytes_allocated: 0,
        bytes_deallocated: 0,
    };
    let mut iteration_count = 0;

    group.bench_function("white_weenie_mirror", |b| {
        b.iter(|| {
            let game_init_fn = || {
                let game_init = GameInitializer::new(&setup.card_db);
                setup.runtime.block_on(async {
                    game_init
                        .init_game(
                            "White Weenie 1".to_string(),
                            &setup.deck1,
                            "White Weenie 2".to_string(),
                            &setup.deck2,
                            20,
                        )
                        .await
                })
            };

            let metrics =
                run_game_with_metrics(black_box(seed), game_init_fn).expect("Game should complete successfully");
            aggregated += metrics.clone();
            iteration_count += 1;
        });
    });

    if iteration_count > 0 {
        print_aggregated_metrics("Old School: White Weenie Mirror", seed, &aggregated, iteration_count);
    }

    group.finish();
}

/// Benchmark: Old School deck matchup - Jeskai Aggro vs Troll Disk
fn bench_game_old_school_jeskai_vs_troll_disk(c: &mut Criterion) {
    ensure_correct_working_directory();

    let setup = match BenchmarkSetup::load(
        "decks/old_school/06_jeskai_aggro_joseantonioprieto.dck",
        "decks/old_school/06_troll_disk_daniellebrunazzo.dck",
    ) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Skipping benchmark - failed to load resources: {e}");
            return;
        }
    };

    let mut group = c.benchmark_group("old_school_matchups");
    group.sample_size(10);
    group.measurement_time(get_benchmark_measurement_time());

    let seed = 42u64;
    let mut aggregated = GameMetrics {
        turns: 0,
        actions: 0,
        duration: Duration::ZERO,
        bytes_allocated: 0,
        bytes_deallocated: 0,
    };
    let mut iteration_count = 0;

    group.bench_function("jeskai_vs_troll_disk", |b| {
        b.iter(|| {
            let game_init_fn = || {
                let game_init = GameInitializer::new(&setup.card_db);
                setup.runtime.block_on(async {
                    game_init
                        .init_game(
                            "Jeskai Aggro".to_string(),
                            &setup.deck1,
                            "Troll Disk".to_string(),
                            &setup.deck2,
                            20,
                        )
                        .await
                })
            };

            let metrics =
                run_game_with_metrics(black_box(seed), game_init_fn).expect("Game should complete successfully");
            aggregated += metrics.clone();
            iteration_count += 1;
        });
    });

    if iteration_count > 0 {
        print_aggregated_metrics(
            "Old School: Jeskai Aggro vs Troll Disk",
            seed,
            &aggregated,
            iteration_count,
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_game_fresh,
    bench_game_fresh_with_logging,
    bench_game_fresh_with_stdout_logging,
    bench_game_snapshot,
    bench_game_rewind,
    bench_game_rewind_play_again,
    bench_game_par_rewind_play_again,        // ENABLED: now uses RewindPlayAgain module
    bench_game_pinned_par_rewind_play_again, // ENABLED: now uses RewindPlayAgain module
    bench_save_snapshot,
    bench_game_old_school_mono_black_vs_the_deck,
    bench_game_old_school_white_weenie_mirror,
    bench_game_old_school_jeskai_vs_troll_disk
);
criterion_main!(benches);
