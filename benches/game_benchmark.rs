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
//! ## Allocator Configuration
//!
//! Use feature flags to select allocator:
//! - `bench-stats-alloc`: stats_alloc with allocation tracking (slower)
//! - `bench-mimalloc`: mimalloc for maximum performance (default)
//!
//! Run with: `cargo bench --features bench-stats-alloc` for tracking
//! Run with: `cargo bench --features bench-mimalloc` for performance (default)

mod allocator;

use allocator::{AllocStats, AllocTracker};
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use mtg_forge_rs::{
    game::{random_controller::RandomController, GameLoop, GameSnapshot, GameState, VerbosityLevel},
    loader::{prefetch_deck_cards, AsyncCardDatabase as CardDatabase, DeckList, DeckLoader, GameInitializer},
    Result,
};
use std::path::PathBuf;
use std::time::Duration;
use tempfile::tempdir;
use tokio::runtime::Runtime;

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

/// Benchmark measurement time in seconds (used by all benchmarks)
const BENCHMARK_TIME_SECS: u64 = 10;

/// Metrics collected during game execution
#[derive(Debug, Clone)]
struct GameMetrics {
    /// Total turns played
    turns: u32,
    /// Total actions (from UndoLog)
    actions: usize,
    /// Game duration
    duration: Duration,
    /// Bytes allocated during game execution
    bytes_allocated: usize,
    /// Bytes deallocated during game execution
    bytes_deallocated: usize,
}

impl GameMetrics {
    /// Calculate actions per second
    fn actions_per_sec(&self) -> f64 {
        self.actions as f64 / self.duration.as_secs_f64()
    }

    /// Calculate turns per second
    fn turns_per_sec(&self) -> f64 {
        self.turns as f64 / self.duration.as_secs_f64()
    }

    /// Calculate average actions per turn
    fn actions_per_turn(&self) -> f64 {
        if self.turns == 0 {
            0.0
        } else {
            self.actions as f64 / self.turns as f64
        }
    }

    /// Calculate net bytes allocated (allocated - deallocated)
    fn net_bytes_allocated(&self) -> i64 {
        self.bytes_allocated as i64 - self.bytes_deallocated as i64
    }

    /// Calculate bytes allocated per turn
    fn bytes_per_turn(&self) -> f64 {
        if self.turns == 0 {
            0.0
        } else {
            self.bytes_allocated as f64 / self.turns as f64
        }
    }

    /// Calculate bytes allocated per second
    fn bytes_per_sec(&self) -> f64 {
        self.bytes_allocated as f64 / self.duration.as_secs_f64()
    }

    /// Calculate average games per second (for aggregated metrics)
    fn avg_games_per_sec(&self, num_games: usize) -> f64 {
        num_games as f64 / self.duration.as_secs_f64()
    }
}

/// Implement addition for GameMetrics to support aggregation
impl std::ops::Add for GameMetrics {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        GameMetrics {
            turns: self.turns + other.turns,
            actions: self.actions + other.actions,
            duration: self.duration + other.duration,
            bytes_allocated: self.bytes_allocated + other.bytes_allocated,
            bytes_deallocated: self.bytes_deallocated + other.bytes_deallocated,
        }
    }
}

impl std::ops::AddAssign for GameMetrics {
    fn add_assign(&mut self, other: Self) {
        self.turns += other.turns;
        self.actions += other.actions;
        self.duration += other.duration;
        self.bytes_allocated += other.bytes_allocated;
        self.bytes_deallocated += other.bytes_deallocated;
    }
}

/// Setup data needed for benchmarking (loaded once, reused across iterations)
struct BenchmarkSetup {
    card_db: CardDatabase,
    deck1: DeckList,
    deck2: DeckList,
    runtime: Runtime,
}

impl BenchmarkSetup {
    fn load(deck1_path: &str, deck2_path: &str) -> Result<Self> {
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

    fn load_same_deck(deck_path: &str) -> Result<Self> {
        Self::load(deck_path, deck_path)
    }
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

/// Run forward gameplay from mid-game snapshot and collect metrics
///
/// This helper function is used by both sequential and parallel rewind benchmarks.
/// It plays the second half of a game from a mid-game state with a specific RNG seed.
///
/// # Parameters
/// - `thread_game`: Mutable reference to the game state (at mid-game point)
/// - `thread_seed`: RNG seed for this playthrough
/// - `rewind_target`: Number of actions at the rewind point (for calculating actions_played)
///
/// # Returns
/// GameMetrics for the forward gameplay (excluding rewind cost)
fn run_forward_gameplay_from_snapshot(
    thread_game: &mut GameState,
    thread_seed: u64,
    rewind_target: usize,
) -> GameMetrics {
    thread_game.seed_rng(thread_seed);

    // Now measure forward gameplay for second half
    let reg = stats_region!();
    let start = std::time::Instant::now();

    let (p1_id, p2_id) = {
        let mut players_iter = thread_game.players.iter().map(|p| p.id);
        (
            players_iter.next().expect("Should have player 1"),
            players_iter.next().expect("Should have player 2"),
        )
    };

    let mut controller1 = RandomController::with_seed(p1_id, thread_seed);
    let mut controller2 = RandomController::with_seed(p2_id, thread_seed);

    let mut game_loop = GameLoop::new(thread_game).with_verbosity(VerbosityLevel::Silent);
    let result = game_loop
        .run_game(&mut controller1, &mut controller2)
        .expect("Game should complete");

    let duration = start.elapsed();
    let stats = get_stats!(reg);

    // Calculate actions played in second half
    let total_actions_now = thread_game.undo_log.len();
    let actions_played = total_actions_now - rewind_target;

    // Record metrics for the forward gameplay only
    GameMetrics {
        turns: result.turns_played,
        actions: actions_played,
        duration,
        bytes_allocated: stats.bytes_allocated,
        bytes_deallocated: stats.bytes_deallocated,
    }
}

/// Helper function to print aggregated metrics
///
/// Note: The "Avg duration/game" shown here is a naive average (total_time / iterations).
/// For accurate per-iteration timing, refer to Criterion's statistical estimate shown above,
/// which accounts for outliers, warmup effects, and provides confidence intervals.
fn print_aggregated_metrics(mode: &str, seed: u64, aggregated: &GameMetrics, iteration_count: usize) {
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

/// Benchmark: Fresh mode - allocate new game each iteration
fn bench_game_fresh(c: &mut Criterion) {
    // Check if test resources exist and load once
    let setup = match BenchmarkSetup::load_same_deck("decks/simple_bolt.dck") {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Skipping benchmark - failed to load resources: {e}");
            return;
        }
    };

    let mut group = c.benchmark_group("game_execution");

    // Configure for long-running benchmarks
    group.sample_size(10); // Reduce sample size since games can be long
    group.measurement_time(Duration::from_secs(BENCHMARK_TIME_SECS));

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
    // Check if test resources exist and load once
    let setup = match BenchmarkSetup::load_same_deck("decks/simple_bolt.dck") {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Skipping benchmark - failed to load resources: {e}");
            return;
        }
    };

    let mut group = c.benchmark_group("game_execution");

    // Configure for long-running benchmarks
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(BENCHMARK_TIME_SECS));

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
    // Check if test resources exist and load once
    let setup = match BenchmarkSetup::load_same_deck("decks/simple_bolt.dck") {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Skipping benchmark - failed to load resources: {e}");
            return;
        }
    };

    let mut group = c.benchmark_group("game_execution");

    // Configure for long-running benchmarks
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(BENCHMARK_TIME_SECS));

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
    // Check if test resources exist and load once
    let setup = match BenchmarkSetup::load_same_deck("decks/simple_bolt.dck") {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Skipping benchmark - failed to load resources: {e}");
            return;
        }
    };

    let mut group = c.benchmark_group("game_execution");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(BENCHMARK_TIME_SECS));

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
    // Check if test resources exist and load once
    let setup = match BenchmarkSetup::load_same_deck("decks/simple_bolt.dck") {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Skipping benchmark - failed to load resources: {e}");
            return;
        }
    };

    let mut group = c.benchmark_group("game_execution");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(BENCHMARK_TIME_SECS));

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
            while game.undo().expect("Undo should succeed") {
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

/// Benchmark: Rewind + replay with different paths
/// Measures forward gameplay after rewind, exploring different game paths
///
/// This benchmark:
/// 1. Plays a full game N turns (done once in setup)
/// 2. For each iteration:
///    - Rewinds to middle of game
///    - Replays second half with new random seed (different path)
/// 3. Measures allocation rate for forward play only (not counting rewind)
///
/// This is comparable to other benchmarks that measure forward gameplay.
fn bench_game_rewind_play_again(c: &mut Criterion) {
    // Check if test resources exist and load once
    let setup = match BenchmarkSetup::load_same_deck("decks/simple_bolt.dck") {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Skipping benchmark - failed to load resources: {e}");
            return;
        }
    };

    let mut group = c.benchmark_group("game_execution");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(BENCHMARK_TIME_SECS));

    let initial_seed = 42u64;

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
    let mut rewind_target = 0;

    group.bench_function("rewind_play_again", |b| {
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

                game.seed_rng(initial_seed);

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
                rewind_target = actions_count / 2; // Rewind to middle of game

                eprintln!("\nRewind + Play Again mode (seed {initial_seed}):");
                eprintln!("  Game completed with {} actions in undo log", actions_count);
                eprintln!(
                    "  Will rewind to action {} (middle) for each iteration...",
                    rewind_target
                );
                eprintln!("  Then replay second half with different random seed per iteration");

                initial_game = Some(game);
            }

            let game = initial_game.as_mut().unwrap();

            // Rewind to middle (outside timing - not counting rewind cost)
            let current_actions = game.undo_log.len();
            let rewinds_needed = current_actions - rewind_target;
            for _ in 0..rewinds_needed {
                game.undo().expect("Undo should succeed");
            }

            // Use different seed for each iteration to explore different paths
            let iteration_seed = initial_seed.wrapping_add(iteration_count as u64);

            // Run forward gameplay using shared helper function
            let metrics = run_forward_gameplay_from_snapshot(game, iteration_seed, rewind_target);

            aggregated += metrics;
            iteration_count += 1;
        });
    });

    if iteration_count > 0 {
        print_aggregated_metrics("Rewind + Play Again", initial_seed, &aggregated, iteration_count);
    }

    group.finish();
}

/// Benchmark: Parallel rewind + replay with different paths (models MCTS parallel search)
/// Measures forward gameplay after rewind across N worker threads
///
/// This benchmark models future MCTS parallel search by:
/// 1. ONE-TIME SETUP: Play a full game to build undo log
/// 2. PER-BATCH SETUP (outside timing): Clone snapshot to N worker threads
/// 3. TIMED LOOP: Each thread runs forward from mid-game, then rewinds for next iteration
///
/// CRITICAL: Uses iter_custom to exclude clone cost from measurements.
/// Only the actual parallel gameplay is timed, not the snapshot cloning.
fn bench_game_par_rewind_play_again(c: &mut Criterion) {
    use rayon::prelude::*;
    use std::time::Instant;

    // Configure rayon to use only physical cores (not hyperthreads)
    let num_physical_cores = num_cpus::get_physical();
    rayon::ThreadPoolBuilder::new()
        .num_threads(num_physical_cores)
        .build_global()
        .ok(); // Ignore error if already initialized

    // Check if test resources exist and load once
    let setup = match BenchmarkSetup::load_same_deck("decks/simple_bolt.dck") {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Skipping benchmark - failed to load resources: {e}");
            return;
        }
    };

    let mut group = c.benchmark_group("game_execution");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(BENCHMARK_TIME_SECS));

    let initial_seed = 42u64;
    let num_threads = num_physical_cores;

    // Track timing for batch logging
    let init_start = Instant::now();

    // ONE-TIME SETUP: Create and play initial game to build undo log
    let game_init = GameInitializer::new(&setup.card_db);
    let mut initial_game = setup
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

    initial_game.seed_rng(initial_seed);

    // Play the game once to build the undo log
    {
        let (p1_id, p2_id) = {
            let mut players_iter = initial_game.players.iter().map(|p| p.id);
            (
                players_iter.next().expect("Should have player 1"),
                players_iter.next().expect("Should have player 2"),
            )
        };

        let mut controller1 = RandomController::with_seed(p1_id, 42);
        let mut controller2 = RandomController::with_seed(p2_id, 42);

        let mut game_loop = GameLoop::new(&mut initial_game).with_verbosity(VerbosityLevel::Silent);
        let _ = game_loop
            .run_game(&mut controller1, &mut controller2)
            .expect("Initial game should complete");
    }

    let actions_count = initial_game.undo_log.len();
    let rewind_target = actions_count / 2; // Rewind to middle of game

    // Rewind to the target position for the benchmark
    while initial_game.undo_log.len() > rewind_target {
        initial_game.undo().expect("Undo should succeed");
    }

    let init_duration = init_start.elapsed();

    eprintln!(
        "\nParallel Rewind + Play Again mode (seed {initial_seed}, {} threads):",
        num_threads
    );
    eprintln!("  Game completed with {} actions in undo log", actions_count);
    eprintln!("  Will start from action {} (middle) for each batch", rewind_target);
    eprintln!("  Then replay second half in parallel with thread-specific RNG seeds");
    eprintln!("  NOTE: Using iter_custom - clone time NOT included in measurements");
    eprintln!("\n=== BATCH TIMING LOG ===");

    let mut batch_number = 0;

    group.bench_function("par_rewind_play_again", |b| {
        b.iter_custom(|iters| {
            batch_number += 1;

            // Log initialization time only for first batch
            if batch_number == 1 {
                eprintln!(
                    "[BATCH-{}] INIT: {:.3}ms (from benchmark start)",
                    batch_number,
                    init_duration.as_secs_f64() * 1000.0
                );
            }

            // PER-BATCH SETUP: Track clone time
            let setup_start = Instant::now();
            // PER-BATCH SETUP (outside timing): Clone snapshots for parallel execution
            // We need to clone before the parallel loop because GameState contains RefCell (not Sync)
            let snapshots: Vec<GameState> = (0..num_threads).map(|_| initial_game.clone()).collect();

            let setup_duration = setup_start.elapsed();
            eprintln!(
                "[BATCH-{}] SETUP: {:.3}ms (clone {} snapshots)",
                batch_number,
                setup_duration.as_secs_f64() * 1000.0,
                num_threads
            );

            // START TIMING - only measure the actual parallel gameplay
            let start = Instant::now();

            // Parallel execution of iters across N threads
            // Each thread does iters/N games
            snapshots
                .into_par_iter()
                .enumerate()
                .for_each(|(thread_id, mut thread_game)| {
                    let games_per_thread = (iters as usize + num_threads - 1) / num_threads; // Ceiling division

                    for i in 0..games_per_thread {
                        if (thread_id * games_per_thread + i) >= iters as usize {
                            break; // Don't overshoot total iters
                        }

                        // Thread-specific seed for this iteration
                        let iter_num = thread_id * games_per_thread + i;
                        let thread_seed = initial_seed.wrapping_add(iter_num as u64);

                        thread_game.seed_rng(thread_seed);

                        // Run forward gameplay from the snapshot
                        let (p1_id, p2_id) = {
                            let mut players_iter = thread_game.players.iter().map(|p| p.id);
                            (
                                players_iter.next().expect("Should have player 1"),
                                players_iter.next().expect("Should have player 2"),
                            )
                        };

                        let mut controller1 = RandomController::with_seed(p1_id, thread_seed);
                        let mut controller2 = RandomController::with_seed(p2_id, thread_seed);

                        let mut game_loop = GameLoop::new(&mut thread_game).with_verbosity(VerbosityLevel::Silent);
                        let _ = game_loop
                            .run_game(&mut controller1, &mut controller2)
                            .expect("Game should complete");

                        // Rewind back to the mid-game state for next iteration
                        while thread_game.undo_log.len() > rewind_target {
                            thread_game.undo().expect("Undo should succeed");
                        }
                    }
                });

            // STOP TIMING
            let batch_duration = start.elapsed();
            eprintln!(
                "[BATCH-{}] EXEC: {:.3}ms ({} iters, {:.3}µs/iter)",
                batch_number,
                batch_duration.as_secs_f64() * 1000.0,
                iters,
                (batch_duration.as_secs_f64() * 1_000_000.0) / iters as f64
            );

            batch_duration
        });
    });

    group.finish();
}

/// Benchmark: Save snapshot to file
fn bench_save_snapshot(c: &mut Criterion) {
    // Check if test resources exist and load once
    let setup = match BenchmarkSetup::load_same_deck("decks/simple_bolt.dck") {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Skipping benchmark - failed to load resources: {e}");
            return;
        }
    };

    let mut group = c.benchmark_group("snapshot_serialization");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(BENCHMARK_TIME_SECS));

    let seed = 42u64;

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

    group.bench_function("save_to_file", |b| {
        b.iter(|| {
            snapshot
                .save_to_file(
                    black_box(&snapshot_path),
                    mtg_forge_rs::game::snapshot::SnapshotFormat::Json,
                )
                .expect("Failed to save snapshot");
        });
    });

    group.finish();
}

/// Benchmark: Old School deck matchup - Mono Black vs The Deck
fn bench_game_old_school_mono_black_vs_the_deck(c: &mut Criterion) {
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
    group.measurement_time(Duration::from_secs(BENCHMARK_TIME_SECS));

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
    let setup = match BenchmarkSetup::load_same_deck("decks/old_school2/white_weenie_classic.dck") {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Skipping benchmark - failed to load resources: {e}");
            return;
        }
    };

    let mut group = c.benchmark_group("old_school_matchups");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(BENCHMARK_TIME_SECS));

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
    group.measurement_time(Duration::from_secs(BENCHMARK_TIME_SECS));

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
    bench_game_par_rewind_play_again,
    bench_save_snapshot,
    bench_game_old_school_mono_black_vs_the_deck,
    bench_game_old_school_white_weenie_mirror,
    bench_game_old_school_jeskai_vs_troll_disk
);
criterion_main!(benches);
