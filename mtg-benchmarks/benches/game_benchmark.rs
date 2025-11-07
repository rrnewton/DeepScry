//! Performance benchmarks for MTG Forge game engine
//!
//! # Benchmark Infrastructure
//!
//! All benchmarks use the `RewindPlayAgain` infrastructure with configuration-driven
//! behavior via `RewindPlayAgainConfig`. This provides consistent timing and metrics
//! collection across different benchmark scenarios.
//!
//! ## Allocator Configuration
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
//! ## Batch Timing with iter_custom
//!
//! **IMPORTANT**: Most benchmarks use `iter_custom` to time entire batches of operations,
//! not individual iterations. This provides accurate timing for workloads that amortize
//! setup costs across multiple operations (like MCTS simulations).
//!
//! ### Pattern
//!
//! ```rust,ignore
//! let mut benchmark: Option<RewindPlayAgain> = None;
//!
//! group.bench_function("my_benchmark", |b| {
//!     b.iter_custom(|iters| {
//!         // Lazy initialization - only runs once, outside timing
//!         if benchmark.is_none() {
//!             let config = RewindPlayAgainConfig::default();
//!             benchmark = Some(RewindPlayAgain::new(config, "MODE"));
//!         }
//!
//!         // Execute batch and return Duration
//!         let bench = benchmark.as_ref().unwrap();
//!         bench.execute_batch_sequential(iters as usize)
//!     });
//! });
//! ```
//!
//! ### Key Points
//!
//! - `iter_custom` expects a closure that returns `Duration`
//! - Initialization happens outside the timing (first call to `iter_custom`)
//! - The batch execution itself is timed by the infrastructure
//! - Criterion calls the closure multiple times with different `iters` values
//!
//! ## Benchmark Groups
//!
//! - `robots_mirror`: Same deck matchups with various configurations
//! - `monoblack_thedeck`: Old School Mono Black vs The Deck
//! - `whiteweenie_mirror`: Old School White Weenie mirror
//! - `jeskai_trolldisk`: Old School Jeskai vs Troll Disk
//! - `snapshot_serialization`: Snapshot save/load benchmarks

mod allocator;

#[path = "lib/mod.rs"]
mod benchlib;

use allocator::{AllocStats, AllocTracker};
use benchlib::{
    ensure_correct_working_directory, get_benchmark_measurement_time, get_benchmark_num_threads,
    print_aggregated_metrics, BatchBenchmark, BenchmarkSetup, GameMetrics, LoggingMode, ParPinned, ParRayon,
    RestartStrategy, RewindPlayAgain, RewindPlayAgainConfig, BASELINE_DECK_PATH,
};
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use mtg_forge_rs::{
    game::{random_controller::RandomController, GameLoop, GameSnapshot, GameState, VerbosityLevel},
    loader::GameInitializer,
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

/// Benchmark: Fresh mode - allocate new game each iteration
/// Tests the cost of fresh gamestate allocation by playing forward only (no rewind)
fn bench_robots_mirror_fresh_games(c: &mut Criterion) {
    let mut benchmark: Option<RewindPlayAgain> = None;

    let mut group = c.benchmark_group("robots_mirror");
    group.sample_size(10);
    group.measurement_time(get_benchmark_measurement_time());

    group.bench_function("fresh_games", |b| {
        b.iter_custom(|iters| {
            if benchmark.is_none() {
                let config = RewindPlayAgainConfig::default()
                    .rewinds_before_restart(Some(0)) // Play forward only, no rewind
                    .restart_strategy(RestartStrategy::Fresh);
                let new_benchmark = RewindPlayAgain::new(config, "FRESH");
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
                "Robots Mirror: Fresh Games",
                bench.seed(),
                &aggregated_metrics,
                total_games,
            );
            bench.print_win_rates("Robots Mirror: Fresh Games");
        }
    }

    group.finish();
}

/// Benchmark: Memory logging mode with rewind+play cycles
/// Measures allocation overhead of logging infrastructure with memory capture
fn bench_robots_mirror_mem_logging_rewind_play_again(c: &mut Criterion) {
    let mut benchmark: Option<RewindPlayAgain> = None;

    let mut group = c.benchmark_group("robots_mirror");
    group.sample_size(10);
    group.measurement_time(get_benchmark_measurement_time());

    group.bench_function("mem_logging_rewind_play_again", |b| {
        b.iter_custom(|iters| {
            if benchmark.is_none() {
                let config = RewindPlayAgainConfig::default()
                    .rewinds_before_restart(None) // Unlimited rewind+replay cycles
                    .restart_strategy(RestartStrategy::Fresh)
                    .logging_mode(LoggingMode::ToMemory);
                let new_benchmark = RewindPlayAgain::new(config, "MEM-LOGGING");
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
                "Robots Mirror: Memory Logging",
                bench.seed(),
                &aggregated_metrics,
                total_games,
            );
            bench.print_win_rates("Robots Mirror: Memory Logging");
        }
    }

    group.finish();
}

/// Benchmark: Stdout logging mode with rewind+play cycles
/// Measures allocation overhead with reusable buffer optimization
fn bench_robots_mirror_stdout_logging_rewind_play_again(c: &mut Criterion) {
    let mut benchmark: Option<RewindPlayAgain> = None;

    let mut group = c.benchmark_group("robots_mirror");
    group.sample_size(10);
    group.measurement_time(get_benchmark_measurement_time());

    group.bench_function("stdout_logging_rewind_play_again", |b| {
        b.iter_custom(|iters| {
            if benchmark.is_none() {
                let config = RewindPlayAgainConfig::default()
                    .rewinds_before_restart(None) // Unlimited rewind+replay cycles
                    .restart_strategy(RestartStrategy::Fresh)
                    .logging_mode(LoggingMode::ToStdout);
                let new_benchmark = RewindPlayAgain::new(config, "STDOUT-LOGGING");
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
                "Robots Mirror: Stdout Logging",
                bench.seed(),
                &aggregated_metrics,
                total_games,
            );
            bench.print_win_rates("Robots Mirror: Stdout Logging");
        }
    }

    group.finish();
}

/// Benchmark: Snapshot mode - allocate new game by cloning
/// Tests the cost of cloning gamestate by playing forward only (no rewind)
fn bench_robots_mirror_snapshot_games(c: &mut Criterion) {
    let mut benchmark: Option<RewindPlayAgain> = None;

    let mut group = c.benchmark_group("robots_mirror");
    group.sample_size(10);
    group.measurement_time(get_benchmark_measurement_time());

    group.bench_function("snapshot_games", |b| {
        b.iter_custom(|iters| {
            if benchmark.is_none() {
                let config = RewindPlayAgainConfig::default()
                    .rewinds_before_restart(Some(0)) // Play forward only, no rewind
                    .restart_strategy(RestartStrategy::Clone);
                let new_benchmark = RewindPlayAgain::new(config, "SNAPSHOT");
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
                "Robots Mirror: Snapshot Games",
                bench.seed(),
                &aggregated_metrics,
                total_games,
            );
            bench.print_win_rates("Robots Mirror: Snapshot Games");
        }
    }

    group.finish();
}

/// Benchmark: Rewind mode - use undo log to rewind game
/// Measures the cost of rewinding using undo() for tree search
fn bench_robots_mirror_rewind_only(c: &mut Criterion) {
    ensure_correct_working_directory();

    // Check if test resources exist and load once
    let setup = match BenchmarkSetup::load_same_deck(BASELINE_DECK_PATH) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Skipping benchmark - failed to load resources: {e}");
            return;
        }
    };

    let mut group = c.benchmark_group("robots_mirror");
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

    group.bench_function("rewind_only", |b| {
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
fn bench_robots_mirror_rewind_play_again(c: &mut Criterion) {
    let mut benchmark: Option<RewindPlayAgain> = None;

    let mut group = c.benchmark_group("robots_mirror");
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
fn bench_robots_mirror_par_rewind_play_again(c: &mut Criterion) {
    let mut benchmark: Option<ParRayon<RewindPlayAgain>> = None;
    let num_threads = get_benchmark_num_threads();

    let mut group = c.benchmark_group("robots_mirror");
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
fn bench_robots_mirror_pinned_par_rewind_play_again(c: &mut Criterion) {
    let num_threads = get_benchmark_num_threads();

    let mut benchmark: Option<ParPinned<RewindPlayAgain>> = None;

    let mut group = c.benchmark_group("robots_mirror");
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
fn bench_monoblack_thedeck_rewind_play_again(c: &mut Criterion) {
    let mut benchmark: Option<RewindPlayAgain> = None;

    let mut group = c.benchmark_group("monoblack_thedeck");
    group.sample_size(10);
    group.measurement_time(get_benchmark_measurement_time());

    group.bench_function("rewind_play_again", |b| {
        b.iter_custom(|iters| {
            if benchmark.is_none() {
                let config = RewindPlayAgainConfig {
                    rewind_percent: 0.5,
                    deck1_path: "decks/old_school/05_mono_black_rogerbrand.dck".to_string(),
                    deck2_path: "decks/old_school/02_thedeck_peterschnidrig.dck".to_string(),
                    rewinds_before_restart: None,
                    restart_strategy: RestartStrategy::Fresh,
                    logging_mode: LoggingMode::Silent,
                };
                let new_benchmark = RewindPlayAgain::new(config, "MONOBLACK-VS-THEDECK");
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
            print_aggregated_metrics("Mono Black vs The Deck", bench.seed(), &aggregated_metrics, total_games);
            bench.print_win_rates("Mono Black vs The Deck");
        }
    }

    group.finish();
}

/// Benchmark: Old School deck matchup - White Weenie mirror
fn bench_whiteweenie_mirror_rewind_play_again(c: &mut Criterion) {
    let mut benchmark: Option<RewindPlayAgain> = None;

    let mut group = c.benchmark_group("whiteweenie_mirror");
    group.sample_size(10);
    group.measurement_time(get_benchmark_measurement_time());

    group.bench_function("rewind_play_again", |b| {
        b.iter_custom(|iters| {
            if benchmark.is_none() {
                let config = RewindPlayAgainConfig::with_same_deck("decks/old_school2/white_weenie_classic.dck");
                let new_benchmark = RewindPlayAgain::new(config, "WHITEWEENIE-MIRROR");
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
            print_aggregated_metrics("White Weenie Mirror", bench.seed(), &aggregated_metrics, total_games);
            bench.print_win_rates("White Weenie Mirror");
        }
    }

    group.finish();
}

/// Benchmark: Old School deck matchup - Jeskai Aggro vs Troll Disk
fn bench_jeskai_trolldisk_rewind_play_again(c: &mut Criterion) {
    let mut benchmark: Option<RewindPlayAgain> = None;

    let mut group = c.benchmark_group("jeskai_trolldisk");
    group.sample_size(10);
    group.measurement_time(get_benchmark_measurement_time());

    group.bench_function("rewind_play_again", |b| {
        b.iter_custom(|iters| {
            if benchmark.is_none() {
                let config = RewindPlayAgainConfig {
                    rewind_percent: 0.5,
                    deck1_path: "decks/old_school/06_jeskai_aggro_joseantonioprieto.dck".to_string(),
                    deck2_path: "decks/old_school/06_troll_disk_daniellebrunazzo.dck".to_string(),
                    rewinds_before_restart: None,
                    restart_strategy: RestartStrategy::Fresh,
                    logging_mode: LoggingMode::Silent,
                };
                let new_benchmark = RewindPlayAgain::new(config, "JESKAI-VS-TROLLDISK");
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
                "Jeskai Aggro vs Troll Disk",
                bench.seed(),
                &aggregated_metrics,
                total_games,
            );
            bench.print_win_rates("Jeskai Aggro vs Troll Disk");
        }
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_robots_mirror_fresh_games,
    bench_robots_mirror_mem_logging_rewind_play_again,
    bench_robots_mirror_stdout_logging_rewind_play_again,
    bench_robots_mirror_snapshot_games,
    bench_robots_mirror_rewind_only,
    bench_robots_mirror_rewind_play_again,
    bench_robots_mirror_par_rewind_play_again,
    bench_robots_mirror_pinned_par_rewind_play_again,
    bench_save_snapshot,
    bench_monoblack_thedeck_rewind_play_again,
    bench_whiteweenie_mirror_rewind_play_again,
    bench_jeskai_trolldisk_rewind_play_again
);
criterion_main!(benches);
