//! Rewind + Play Again benchmark infrastructure
//!
//! This module provides the `RewindPlayAgain` struct for benchmarking MCTS-style
//! game simulation where games are played forward from a midpoint and then rewound
//! back to explore different paths.

use crate::allocator::{AllocStats, AllocTracker};
use crate::{create_midgame_state, BenchmarkSetup, GameMetrics, BASELINE_DECK_PATH};
use mtg_forge_rs::game::{random_controller::RandomController, GameLoop, GameState, VerbosityLevel};
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};
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

/// Game outcome for win rate tracking
#[derive(Debug, Clone, Copy)]
pub enum GameOutcome {
    Player1Win,
    Player2Win,
}

/// Benchmark state for rewind + play again benchmarks
///
/// Consolidates shared logic across sequential and parallel variants.
/// Tracks win rates and provides methods for executing batches of games.
/// All metrics use Arc-wrapped atomic types for thread-safe parallel execution.
pub struct RewindPlayAgain {
    /// The mid-game template (at 50% point, undo log cleared)
    midgame_template: GameState,
    /// RNG seed used for game initialization
    seed: u64,
    /// Player 1 win count (Arc-wrapped atomic for thread-safe updates)
    p1_wins: Arc<AtomicUsize>,
    /// Player 2 win count (Arc-wrapped atomic for thread-safe updates)
    p2_wins: Arc<AtomicUsize>,
    /// Total games played (Arc-wrapped atomic for thread-safe updates)
    total_games: Arc<AtomicUsize>,
    /// Aggregated turns across all games (Arc-wrapped atomic for thread-safe updates)
    total_turns: Arc<AtomicU32>,
    /// Aggregated actions across all games (Arc-wrapped atomic for thread-safe updates)
    total_actions: Arc<AtomicUsize>,
    /// Aggregated duration in nanoseconds (Arc-wrapped atomic for thread-safe updates)
    total_duration_nanos: Arc<AtomicU64>,
    /// Aggregated bytes allocated (Arc-wrapped atomic for thread-safe updates)
    total_bytes_allocated: Arc<AtomicUsize>,
    /// Aggregated bytes deallocated (Arc-wrapped atomic for thread-safe updates)
    total_bytes_deallocated: Arc<AtomicUsize>,
}

impl RewindPlayAgain {
    /// Create a new RewindPlayAgain benchmark state
    ///
    /// This creates a mid-game state by playing to 50% and clearing the undo log.
    ///
    /// # Parameters
    /// - `mode_tag`: Mode description (e.g., "SEQUENTIAL", "PARALLEL")
    pub fn new(mode_tag: &str) -> Self {
        use crate::ensure_correct_working_directory;

        let seed = 42u64;
        ensure_correct_working_directory();
        let setup = match BenchmarkSetup::load_same_deck(BASELINE_DECK_PATH) {
            Ok(s) => s,
            Err(e) => {
                panic!("Benchmark failed to load resources: {e}");
            }
        };

        eprintln!(
            "\nRewind + Play Again mode (seed {seed}, {mode_tag}), deck {}:",
            BASELINE_DECK_PATH
        );
        let (midgame_template, original_total_actions) = create_midgame_state(&setup, seed);
        eprintln!("  Full game had {} actions", original_total_actions);
        eprintln!("  Starting from midpoint (undo log cleared)");
        eprintln!("  Will execute batches of (play forward + rewind) cycles");
        eprintln!("  NOTE: Each iteration rewinds, NO cloning!");

        Self {
            midgame_template,
            seed,
            p1_wins: Arc::new(AtomicUsize::new(0)),
            p2_wins: Arc::new(AtomicUsize::new(0)),
            total_games: Arc::new(AtomicUsize::new(0)),
            total_turns: Arc::new(AtomicU32::new(0)),
            total_actions: Arc::new(AtomicUsize::new(0)),
            total_duration_nanos: Arc::new(AtomicU64::new(0)),
            total_bytes_allocated: Arc::new(AtomicUsize::new(0)),
            total_bytes_deallocated: Arc::new(AtomicUsize::new(0)),
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
        self.total_games.fetch_add(batch_size, Ordering::Relaxed);
        self.total_turns.fetch_add(batch_turns, Ordering::Relaxed);
        self.total_actions.fetch_add(batch_actions, Ordering::Relaxed);
        self.total_duration_nanos
            .fetch_add(duration.as_nanos() as u64, Ordering::Relaxed);
        self.total_bytes_allocated
            .fetch_add(stats.bytes_allocated, Ordering::Relaxed);
        self.total_bytes_deallocated
            .fetch_add(stats.bytes_deallocated, Ordering::Relaxed);

        duration
    }

    /// Execute a batch of games in parallel using Rayon
    ///
    /// Each thread gets its own game state clone and runs a portion of the batch.
    /// Each thread tracks allocations locally, then atomically aggregates results.
    ///
    /// # Parameters
    /// - `batch_size`: Total number of games to play across all threads
    /// - `num_threads`: Number of parallel worker threads
    ///
    /// # Returns
    /// Duration of the parallel batch execution
    pub fn execute_batch_parallel(&self, batch_size: usize, num_threads: usize) -> Duration {
        use rayon::prelude::*;

        // Extract values we need before entering parallel context
        let base_seed = self.seed;
        let p1_wins = &self.p1_wins;
        let p2_wins = &self.p2_wins;
        let total_games = &self.total_games;
        let total_turns = &self.total_turns;
        let total_actions = &self.total_actions;
        let total_duration_nanos = &self.total_duration_nanos;
        let total_bytes_allocated = &self.total_bytes_allocated;
        let total_bytes_deallocated = &self.total_bytes_deallocated;

        // Clone game states for each thread OUTSIDE timing
        let snapshots: Vec<GameState> = (0..num_threads).map(|_| self.midgame_template.clone()).collect();

        // START TIMING - only measure the actual parallel gameplay
        let start = std::time::Instant::now();

        // Parallel execution across N threads
        // Each thread does batch_size/N games, reusing its game state with rewind
        snapshots
            .into_par_iter()
            .enumerate()
            .for_each(|(thread_id, mut thread_game)| {
                let games_per_thread = batch_size.div_ceil(num_threads);

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

                    let mut game_loop = GameLoop::new(&mut thread_game).with_verbosity(VerbosityLevel::Silent);
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
                total_games.fetch_add(actual_games, Ordering::Relaxed);
                total_turns.fetch_add(batch_turns, Ordering::Relaxed);
                total_actions.fetch_add(batch_actions, Ordering::Relaxed);
                total_duration_nanos.fetch_add(batch_duration.as_nanos() as u64, Ordering::Relaxed);
                total_bytes_allocated.fetch_add(stats.bytes_allocated, Ordering::Relaxed);
                total_bytes_deallocated.fetch_add(stats.bytes_deallocated, Ordering::Relaxed);
            });

        // STOP TIMING
        start.elapsed()
    }

    /// Get aggregated metrics collected so far
    ///
    /// Returns a GameMetrics struct with all accumulated metrics.
    /// Reads atomic values with Relaxed ordering.
    pub fn get_aggregated_metrics(&self) -> GameMetrics {
        GameMetrics {
            turns: self.total_turns.load(Ordering::Relaxed),
            actions: self.total_actions.load(Ordering::Relaxed),
            duration: Duration::from_nanos(self.total_duration_nanos.load(Ordering::Relaxed)),
            bytes_allocated: self.total_bytes_allocated.load(Ordering::Relaxed),
            bytes_deallocated: self.total_bytes_deallocated.load(Ordering::Relaxed),
        }
    }

    /// Get total number of games played
    pub fn get_total_games(&self) -> usize {
        use std::sync::atomic::Ordering;
        self.total_games.load(Ordering::Relaxed)
    }

    /// Execute a batch of games in parallel using pinned threads
    ///
    /// Each thread gets its own game state clone and runs a portion of the batch.
    /// Uses custom thread pool with spin barriers for microsecond-accurate timing.
    ///
    /// # Parameters
    /// - `batch_size`: Total number of games to play across all threads
    /// - `num_threads`: Number of parallel worker threads
    ///
    /// # Returns
    /// Duration of the parallel batch execution
    pub fn execute_batch_pinned_parallel(&self, batch_size: usize, num_threads: usize) -> Duration {
        use crate::pinned_thread_pool::execute_parallel_batch;

        // Clone Arc-wrapped atomics so they can be moved into the closure
        let base_seed = self.seed;
        let p1_wins = Arc::clone(&self.p1_wins);
        let p2_wins = Arc::clone(&self.p2_wins);
        let total_games = Arc::clone(&self.total_games);
        let total_turns = Arc::clone(&self.total_turns);
        let total_actions = Arc::clone(&self.total_actions);
        let total_duration_nanos = Arc::clone(&self.total_duration_nanos);
        let total_bytes_allocated = Arc::clone(&self.total_bytes_allocated);
        let total_bytes_deallocated = Arc::clone(&self.total_bytes_deallocated);

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
                total_games.fetch_add(actual_games, Ordering::Relaxed);
                total_turns.fetch_add(batch_turns, Ordering::Relaxed);
                total_actions.fetch_add(batch_actions, Ordering::Relaxed);
                total_duration_nanos.fetch_add(batch_duration.as_nanos() as u64, Ordering::Relaxed);
                total_bytes_allocated.fetch_add(stats.bytes_allocated, Ordering::Relaxed);
                total_bytes_deallocated.fetch_add(stats.bytes_deallocated, Ordering::Relaxed);
            });

        batch_duration
    }

    /// Print win rate analysis for this benchmark
    pub fn print_win_rates(&self, mode: &str) {
        use std::sync::atomic::Ordering;

        let total = self.total_games.load(Ordering::Relaxed);
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
