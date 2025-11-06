//! Core benchmark implementations
//!
//! This module provides the `RewindPlayAgain` benchmark and the `ParRayon` wrapper
//! for parallelizing batch benchmarks.

use super::super::allocator::{AllocStats, AllocTracker};
use super::types::{AtomicMetrics, BatchBenchmark, GameMetrics, GameOutcome, RewindPlayAgainConfig};
use super::utils::{create_game_at_percent, ensure_correct_working_directory, BenchmarkSetup};
use mtg_forge_rs::game::{random_controller::RandomController, GameLoop, GameState, VerbosityLevel};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

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

/// Benchmark state for rewind + play again benchmarks
///
/// Consolidates shared logic across sequential and parallel variants.
/// Tracks win rates and provides methods for executing batches of games.
/// Uses Arc<AtomicMetrics> for thread-safe metric aggregation.
pub struct RewindPlayAgain {
    /// Configuration for this benchmark
    #[allow(dead_code)] // Will be used for rounds_before_restart logic
    config: RewindPlayAgainConfig,
    /// The initial full game state (before any rewind)
    #[allow(dead_code)] // Will be used for restart logic
    initial_game_template: GameState,
    /// The mid-game template (at rewind_percent point, undo log cleared)
    midgame_template: GameState,
    /// RNG seed used for game initialization
    seed: u64,
    /// Player 1 win count (Arc-wrapped atomic for thread-safe updates)
    p1_wins: Arc<AtomicUsize>,
    /// Player 2 win count (Arc-wrapped atomic for thread-safe updates)
    p2_wins: Arc<AtomicUsize>,
    /// Aggregated game metrics (Arc-wrapped for cheap cloning across threads)
    metrics: Arc<AtomicMetrics>,
    /// Number of rounds completed (for tracking restart logic)
    #[allow(dead_code)] // Will be used for rounds_before_restart logic
    rounds_completed: Arc<AtomicUsize>,
}

// SAFETY: RewindPlayAgain is Sync because:
// 1. The midgame_template (GameState) is only ever cloned, never shared across threads
// 2. All Arc fields (p1_wins, p2_wins, metrics) are already Sync
// 3. The seed (u64) is Copy and immutable
// ParRayon only shares &RewindPlayAgain across threads, and each thread clones the game state
unsafe impl Sync for RewindPlayAgain {}

impl RewindPlayAgain {
    /// Create a new RewindPlayAgain benchmark state
    ///
    /// This creates game states according to the provided configuration.
    ///
    /// # Parameters
    /// - `config`: Configuration for the benchmark
    /// - `mode_tag`: Mode description (e.g., "SEQUENTIAL", "PARALLEL")
    pub fn new(config: RewindPlayAgainConfig, mode_tag: &str) -> Self {
        let seed = 43u64;
        ensure_correct_working_directory();
        let setup = match BenchmarkSetup::load(&config.deck1_path, &config.deck2_path) {
            Ok(s) => s,
            Err(e) => {
                panic!("Benchmark failed to load resources: {e}");
            }
        };

        eprintln!(
            "\nRewind + Play Again mode (seed {seed}, {mode_tag}), decks {} vs {}:",
            config.deck1_path, config.deck2_path
        );
        eprintln!("  Rewind percent: {:.1}%", config.rewind_percent * 100.0);
        eprintln!(
            "  Rounds before restart: {}",
            match config.rounds_before_restart {
                None => "infinite".to_string(),
                Some(0) => "0 (forward only)".to_string(),
                Some(n) => n.to_string(),
            }
        );
        eprintln!("  Restart strategy: {:?}", config.restart_strategy);

        let (initial_game_template, midgame_template, original_total_actions) =
            create_game_at_percent(&setup, seed, config.rewind_percent);

        eprintln!("  Full game had {} actions", original_total_actions);
        eprintln!(
            "  Starting from {:.1}% point (undo log cleared)",
            config.rewind_percent * 100.0
        );

        if config.rounds_before_restart == Some(0) {
            eprintln!("  Will play forward only (no rewind)");
        } else {
            eprintln!("  Will execute batches of (play forward + rewind) cycles");
        }

        Self {
            config,
            initial_game_template,
            midgame_template,
            seed,
            p1_wins: Arc::new(AtomicUsize::new(0)),
            p2_wins: Arc::new(AtomicUsize::new(0)),
            metrics: Arc::new(AtomicMetrics::new()),
            rounds_completed: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Get the seed used for this benchmark
    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// Execute a single game from midpoint to end and rewind back
    ///
    /// This plays forward from the mid-game state, then rewinds back to midpoint.
    /// The complete cycle (forward + rewind) is what matters for MCTS simulation.
    ///
    /// This method only captures metrics that would be lost after rewind (turns, actions).
    /// Duration and allocation tracking should be done at the batch level.
    ///
    /// # Parameters
    /// - `game`: Mutable reference to game state (will be mutated and rewound)
    /// - `seed`: RNG seed for this playthrough
    ///
    /// # Returns
    /// Tuple of (turns played, actions performed, GameOutcome)
    pub fn execute_single_game(&self, game: &mut GameState, seed: u64) -> (u32, usize, GameOutcome) {
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

    /// Execute a batch of games sequentially
    ///
    /// This is the correct benchmark workload for sequential rewind+play:
    /// For each iteration: (1) play forward from midpoint, (2) rewind to midpoint, (3) repeat
    ///
    /// Timing and allocation tracking are done at the batch level (thread-local stats_alloc).
    /// Batch-level metrics are then atomically aggregated into shared state.
    ///
    /// # Parameters
    /// - `batch_size`: Number of games to play in this batch
    ///
    /// # Returns
    /// Duration of the batch (for Criterion)
    pub fn execute_batch_sequential(&self, batch_size: usize) -> Duration {
        // Start with a single working copy that we'll reuse
        let mut game = self.midgame_template.clone();

        // Track metrics for this batch (thread-local)
        let mut batch_turns = 0u32;
        let mut batch_actions = 0usize;

        // Start timing the entire batch (forward + rewind for all iterations)
        let reg = stats_region!();
        let start = std::time::Instant::now();

        for i in 0..batch_size {
            let seed = self.seed.wrapping_add(i as u64);

            // Execute game (forward + rewind) and collect metrics
            let (turns_played, actions_played, outcome) = self.execute_single_game(&mut game, seed);

            // Aggregate metrics for this batch (thread-local)
            batch_turns += turns_played;
            batch_actions += actions_played;

            // Update win counters atomically
            match outcome {
                GameOutcome::Player1Win => {
                    self.p1_wins.fetch_add(1, Ordering::Relaxed);
                }
                GameOutcome::Player2Win => {
                    self.p2_wins.fetch_add(1, Ordering::Relaxed);
                }
            }
        }

        // Stop timing and collect allocation stats (batch-level)
        let duration = start.elapsed();
        let stats = get_stats!(reg);

        // Atomically update all aggregated metrics from this batch
        self.metrics
            .add_batch(batch_size, batch_turns, batch_actions, duration, &stats);

        duration
    }

    /// Get aggregated metrics collected so far
    ///
    /// Returns a GameMetrics struct with all accumulated metrics.
    /// Reads atomic values with Relaxed ordering.
    pub fn get_aggregated_metrics(&self) -> GameMetrics {
        self.metrics.to_game_metrics()
    }

    /// Get total number of games played
    pub fn get_total_games(&self) -> usize {
        self.metrics.get_total_games()
    }

    /// Execute a batch of games in parallel using pinned threads
    ///
    /// Each thread gets its own game state clone and runs a portion of the batch.
    /// Uses custom thread pool with spin barriers for microsecond-accurate timing.
    ///
    /// This method is used internally by ParPinned wrapper. Use ParPinned for
    /// parallel execution via the BatchBenchmark trait.
    ///
    /// # Parameters
    /// - `batch_size`: Total number of games to play across all threads
    /// - `num_threads`: Number of parallel worker threads
    ///
    /// # Returns
    /// Duration of the parallel batch execution
    #[allow(dead_code)] // Used by ParPinned wrapper
    pub(crate) fn execute_batch_pinned_parallel(&self, batch_size: usize, num_threads: usize) -> Duration {
        use super::super::pinned_thread_pool::execute_parallel_batch;

        // Clone Arc references for the closure
        let base_seed = self.seed;
        let p1_wins = Arc::clone(&self.p1_wins);
        let p2_wins = Arc::clone(&self.p2_wins);
        let metrics = Arc::clone(&self.metrics);

        let games_per_thread = batch_size.div_ceil(num_threads);

        // Execute parallel batch with pinned threads
        // Each thread gets a clone of the midgame template and reuses it with rewind
        let (batch_duration, _results) =
            execute_parallel_batch(num_threads, &self.midgame_template, move |thread_id, thread_game| {
                // Track metrics for this thread's batch (thread-local)
                let mut batch_turns = 0u32;
                let mut batch_actions = 0usize;

                // Track allocations for this thread's batch
                let reg = stats_region!();
                let batch_start = std::time::Instant::now();

                for i in 0..games_per_thread {
                    let game_idx = thread_id * games_per_thread + i;
                    if game_idx >= batch_size {
                        break; // Don't overshoot total batch size
                    }

                    let seed = base_seed.wrapping_add(game_idx as u64);

                    // Execute single game (forward + rewind) - inline the logic here
                    // to avoid needing &self in the closure
                    thread_game.seed_rng(seed);

                    let start_turn = thread_game.turn.turn_number;
                    let start_undo_size = thread_game.undo_log.len();

                    let (p1_id, p2_id) = {
                        let mut players_iter = thread_game.players.iter().map(|p| p.id);
                        (
                            players_iter.next().expect("Should have player 1"),
                            players_iter.next().expect("Should have player 2"),
                        )
                    };

                    let mut controller1 = RandomController::with_seed(p1_id, seed);
                    let mut controller2 = RandomController::with_seed(p2_id, seed);

                    let mut game_loop = GameLoop::new(thread_game).with_verbosity(VerbosityLevel::Silent);
                    let result = game_loop
                        .run_game(&mut controller1, &mut controller2)
                        .expect("Game should complete");

                    // Capture metrics BEFORE rewinding
                    let end_turn = thread_game.turn.turn_number;
                    let turns_played = end_turn.saturating_sub(start_turn);
                    let actions_played = thread_game.undo_log.len() - start_undo_size;

                    // Determine winner
                    let outcome = if result.winner == Some(p1_id) {
                        GameOutcome::Player1Win
                    } else {
                        GameOutcome::Player2Win
                    };

                    // Rewind back to midpoint
                    while thread_game.undo_log.len() > start_undo_size {
                        thread_game.undo().expect("Undo should succeed");
                    }

                    // Aggregate metrics for this thread (thread-local)
                    batch_turns += turns_played;
                    batch_actions += actions_played;

                    // Update win counters atomically
                    match outcome {
                        GameOutcome::Player1Win => {
                            p1_wins.fetch_add(1, Ordering::Relaxed);
                        }
                        GameOutcome::Player2Win => {
                            p2_wins.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }

                // Stop timing and collect allocation stats for this thread
                let batch_duration = batch_start.elapsed();
                let stats = get_stats!(reg);

                // Atomically aggregate this thread's metrics into shared state
                let actual_games = games_per_thread.min(batch_size - thread_id * games_per_thread);
                metrics.add_batch(actual_games, batch_turns, batch_actions, batch_duration, &stats);
            });

        batch_duration
    }

    /// Print win rate analysis for this benchmark
    pub fn print_win_rates(&self, mode: &str) {
        let total = self.metrics.get_total_games();
        if total == 0 {
            return;
        }

        let p1_wins = self.p1_wins.load(Ordering::Relaxed);
        let p2_wins = self.p2_wins.load(Ordering::Relaxed);

        eprintln!(
            "\n=== Win Rate Analysis - {mode} (seed {}, {total} games) ===",
            self.seed
        );
        eprintln!("  P1 wins: {} ({:.1}%)", p1_wins, 100.0 * p1_wins as f64 / total as f64);
        eprintln!("  P2 wins: {} ({:.1}%)", p2_wins, 100.0 * p2_wins as f64 / total as f64);
    }
}

/// Implement BatchBenchmark for RewindPlayAgain (sequential execution only)
impl BatchBenchmark for RewindPlayAgain {
    fn execute_batch(&self, batch_size: usize, num_threads: usize) -> Result<Duration, String> {
        if num_threads != 1 {
            return Err(format!(
                "RewindPlayAgain only supports sequential execution (num_threads=1), got {}",
                num_threads
            ));
        }
        Ok(self.execute_batch_sequential(batch_size))
    }

    fn get_metrics(&self) -> GameMetrics {
        self.get_aggregated_metrics()
    }

    fn total_games(&self) -> usize {
        self.get_total_games()
    }
}

/// Parallel wrapper using Rayon for batch benchmark execution
///
/// Wraps any BatchBenchmark implementation and provides parallel execution
/// using Rayon's thread pool. The wrapped benchmark must support sequential
/// execution (num_threads=1).
#[allow(dead_code)] // Infrastructure for future use
pub struct ParRayon<T> {
    inner: T,
}

#[allow(dead_code)] // Infrastructure for future use
impl<T> ParRayon<T> {
    /// Create a new parallel wrapper around a sequential benchmark
    pub fn new(inner: T) -> Self {
        ParRayon { inner }
    }

    /// Get a reference to the inner benchmark
    pub fn inner(&self) -> &T {
        &self.inner
    }
}

impl<T: BatchBenchmark + Sync> BatchBenchmark for ParRayon<T> {
    fn execute_batch(&self, batch_size: usize, num_threads: usize) -> Result<Duration, String> {
        use rayon::prelude::*;

        if num_threads < 1 {
            return Err(format!("num_threads must be >= 1, got {}", num_threads));
        }

        // For single-threaded execution, just delegate to the inner benchmark
        if num_threads == 1 {
            return self.inner.execute_batch(batch_size, 1);
        }

        // Calculate iterations per thread
        let iters_per_thread = batch_size.div_ceil(num_threads);

        // Start timing
        let start = std::time::Instant::now();

        // Execute in parallel using Rayon
        // Each thread calls the inner benchmark's sequential execute_batch
        (0..num_threads)
            .into_par_iter()
            .try_for_each(|thread_id| -> Result<(), String> {
                let thread_iters = if thread_id == num_threads - 1 {
                    // Last thread handles any remainder
                    batch_size - (thread_id * iters_per_thread)
                } else {
                    iters_per_thread
                };

                if thread_iters > 0 {
                    self.inner.execute_batch(thread_iters, 1)?;
                }
                Ok(())
            })?;

        Ok(start.elapsed())
    }

    fn get_metrics(&self) -> GameMetrics {
        self.inner.get_metrics()
    }

    fn total_games(&self) -> usize {
        self.inner.total_games()
    }
}

/// Parallel wrapper using pinned threads for batch benchmark execution
///
/// Wraps RewindPlayAgain and provides parallel execution using pinned threads
/// with spin barriers for microsecond-accurate timing.
#[allow(dead_code)] // Infrastructure for future use
pub struct ParPinned {
    inner: RewindPlayAgain,
}

#[allow(dead_code)] // Infrastructure for future use
impl ParPinned {
    /// Create a new pinned-parallel wrapper around a RewindPlayAgain benchmark
    pub fn new(inner: RewindPlayAgain) -> Self {
        ParPinned { inner }
    }

    /// Get a reference to the inner benchmark
    pub fn inner(&self) -> &RewindPlayAgain {
        &self.inner
    }
}

impl BatchBenchmark for ParPinned {
    fn execute_batch(&self, batch_size: usize, num_threads: usize) -> Result<Duration, String> {
        if num_threads < 1 {
            return Err(format!("num_threads must be >= 1, got {}", num_threads));
        }

        // For single-threaded execution, delegate to sequential execution
        if num_threads == 1 {
            return Ok(self.inner.execute_batch_sequential(batch_size));
        }

        // For parallel execution, use the pinned parallel implementation
        Ok(self.inner.execute_batch_pinned_parallel(batch_size, num_threads))
    }

    fn get_metrics(&self) -> GameMetrics {
        self.inner.get_aggregated_metrics()
    }

    fn total_games(&self) -> usize {
        self.inner.get_total_games()
    }
}
