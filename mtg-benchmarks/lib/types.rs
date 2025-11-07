//! Type definitions for benchmark infrastructure. Constants too.
//!
//! This module defines core types and traits used across the benchmark suite,
//! including metrics tracking, batch execution, and game outcomes.

use super::allocator::AllocStats;
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

/// Use this for most benchmarks for a performance baseline.
pub const BASELINE_DECK_PATH: &str = "decks/old_school/03_robots_jesseisbak.dck";

/// Metrics collected during game execution
#[derive(Debug, Clone)]
pub struct GameMetrics {
    /// Total turns played
    pub turns: u32,
    /// Total actions (from UndoLog)
    pub actions: usize,
    /// Game duration
    pub duration: Duration,
    /// Bytes allocated during game execution
    pub bytes_allocated: usize,
    /// Bytes deallocated during game execution
    pub bytes_deallocated: usize,
}

#[allow(dead_code)] // Used by benchmarks, not all binaries
impl GameMetrics {
    /// Calculate actions per second
    pub fn actions_per_sec(&self) -> f64 {
        self.actions as f64 / self.duration.as_secs_f64()
    }

    /// Calculate turns per second
    pub fn turns_per_sec(&self) -> f64 {
        self.turns as f64 / self.duration.as_secs_f64()
    }

    /// Calculate average actions per turn
    pub fn actions_per_turn(&self) -> f64 {
        if self.turns == 0 {
            0.0
        } else {
            self.actions as f64 / self.turns as f64
        }
    }

    /// Calculate net bytes allocated (allocated - deallocated)
    pub fn net_bytes_allocated(&self) -> i64 {
        self.bytes_allocated as i64 - self.bytes_deallocated as i64
    }

    /// Calculate bytes allocated per turn
    pub fn bytes_per_turn(&self) -> f64 {
        if self.turns == 0 {
            0.0
        } else {
            self.bytes_allocated as f64 / self.turns as f64
        }
    }

    /// Calculate bytes allocated per second
    pub fn bytes_per_sec(&self) -> f64 {
        self.bytes_allocated as f64 / self.duration.as_secs_f64()
    }

    /// Calculate average games per second (for aggregated metrics)
    pub fn avg_games_per_sec(&self, num_games: usize) -> f64 {
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

#[derive(Debug)]
/// Atomic version of GameMetrics for thread-safe aggregation
///
/// Mirrors GameMetrics but uses atomic types for concurrent updates.
/// Wrapped in Arc for cheap cloning across threads.
///
/// # Duration Tracking
/// - `total_duration_nanos`: Wall-clock time (set by parallel executor, not additive)
/// - `total_core_nanos`: CPU time (sum of thread durations, additive across threads)
pub struct AtomicMetrics {
    /// Total games played
    pub total_games: AtomicUsize,
    /// Aggregated turns across all games
    pub total_turns: AtomicU32,
    /// Aggregated actions across all games
    pub total_actions: AtomicUsize,
    /// Wall-clock duration in nanoseconds (overwritten by parallel executor)
    pub total_duration_nanos: AtomicU64,
    /// CPU time in nanoseconds (sum of thread durations, for parallel efficiency tracking)
    pub total_core_nanos: AtomicU64,
    /// Aggregated bytes allocated
    pub total_bytes_allocated: AtomicUsize,
    /// Aggregated bytes deallocated
    pub total_bytes_deallocated: AtomicUsize,
}

impl AtomicMetrics {
    /// Create new AtomicMetrics with all values initialized to zero
    pub fn new() -> Self {
        AtomicMetrics {
            total_games: AtomicUsize::new(0),
            total_turns: AtomicU32::new(0),
            total_actions: AtomicUsize::new(0),
            total_duration_nanos: AtomicU64::new(0),
            total_core_nanos: AtomicU64::new(0),
            total_bytes_allocated: AtomicUsize::new(0),
            total_bytes_deallocated: AtomicUsize::new(0),
        }
    }

    /// Reset all metrics to zero
    ///
    /// This is used to clear metrics before running a batch, ensuring that
    /// only the most recent batch's measurements are captured.
    pub fn reset(&self) {
        self.total_games.store(0, Ordering::Relaxed);
        self.total_turns.store(0, Ordering::Relaxed);
        self.total_actions.store(0, Ordering::Relaxed);
        self.total_duration_nanos.store(0, Ordering::Relaxed);
        self.total_core_nanos.store(0, Ordering::Relaxed);
        self.total_bytes_allocated.store(0, Ordering::Relaxed);
        self.total_bytes_deallocated.store(0, Ordering::Relaxed);
    }

    /// Atomically add metrics from a batch
    ///
    /// For sequential execution, duration represents both wall time and CPU time.
    /// For parallel execution, this adds CPU time; wall time must be set separately.
    pub fn add_batch(&self, games: usize, turns: u32, actions: usize, duration: Duration, alloc_stats: &AllocStats) {
        self.total_games.fetch_add(games, Ordering::Relaxed);
        self.total_turns.fetch_add(turns, Ordering::Relaxed);
        self.total_actions.fetch_add(actions, Ordering::Relaxed);

        // Add to both duration and core time for sequential execution
        // Parallel executor will overwrite duration with wall time
        let nanos = duration.as_nanos() as u64;
        self.total_duration_nanos.fetch_add(nanos, Ordering::Relaxed);
        self.total_core_nanos.fetch_add(nanos, Ordering::Relaxed);

        self.total_bytes_allocated
            .fetch_add(alloc_stats.bytes_allocated, Ordering::Relaxed);
        self.total_bytes_deallocated
            .fetch_add(alloc_stats.bytes_deallocated, Ordering::Relaxed);
    }

    /// Set the wall-clock duration (used by parallel executors)
    ///
    /// This overwrites the total_duration_nanos with the actual wall-clock time,
    /// while total_core_nanos retains the sum of thread durations (CPU time).
    pub fn set_wall_time(&self, duration: Duration) {
        self.total_duration_nanos.store(duration.as_nanos() as u64, Ordering::Relaxed);
    }

    /// Convert to GameMetrics snapshot by loading all atomic values
    pub fn to_game_metrics(&self) -> GameMetrics {
        GameMetrics {
            turns: self.total_turns.load(Ordering::Relaxed),
            actions: self.total_actions.load(Ordering::Relaxed),
            duration: Duration::from_nanos(self.total_duration_nanos.load(Ordering::Relaxed)),
            bytes_allocated: self.total_bytes_allocated.load(Ordering::Relaxed),
            bytes_deallocated: self.total_bytes_deallocated.load(Ordering::Relaxed),
        }
    }

    /// Get total games played
    pub fn get_total_games(&self) -> usize {
        self.total_games.load(Ordering::Relaxed)
    }

    /// Get CPU time (core-seconds) for parallel efficiency calculation
    pub fn get_core_seconds(&self) -> f64 {
        self.total_core_nanos.load(Ordering::Relaxed) as f64 / 1_000_000_000.0
    }
}

impl Default for AtomicMetrics {
    fn default() -> Self {
        Self::new()
    }
}

/// Game outcome for win rate tracking
#[derive(Debug, Clone, Copy)]
pub enum GameOutcome {
    Player1Win,
    Player2Win,
}

/// Strategy for reinitializing games when restarting from scratch
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // Will be used when restart logic is implemented
pub enum RestartStrategy {
    /// Clone the initial game state
    Clone,
    /// Create a fresh game state from scratch
    Fresh,
}

/// Logging mode for game execution
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // ToMemory and ToStdout will be used for specialized benchmarks
pub enum LoggingMode {
    /// No logging output (VerbosityLevel::Silent)
    Silent,
    /// Capture logs to memory (VerbosityLevel::Normal with capture enabled)
    ToMemory,
    /// Write logs to stdout (VerbosityLevel::Normal without capture)
    ToStdout,
}

/// Configuration for RewindPlayAgain benchmark
#[derive(Debug, Clone)]
pub struct RewindPlayAgainConfig {
    /// Percentage of game to play before rewinding (0.0 to 1.0, default 0.5)
    pub rewind_percent: f64,
    /// Path to player 1's deck
    pub deck1_path: String,
    /// Path to player 2's deck
    pub deck2_path: String,
    /// Number of rewind+replay rounds before restarting from scratch
    /// - None = infinite (never restart)
    /// - Some(0) = play forward only, no rewind
    /// - Some(n) = rewind n times then restart
    pub rewinds_before_restart: Option<usize>,
    /// How to reinitialize when restarting (only relevant if rewinds_before_restart is Some)
    pub restart_strategy: RestartStrategy,
    /// Logging mode for game execution
    pub logging_mode: LoggingMode,
}

impl Default for RewindPlayAgainConfig {
    fn default() -> Self {
        RewindPlayAgainConfig {
            rewind_percent: 0.5,
            deck1_path: BASELINE_DECK_PATH.to_string(),
            deck2_path: BASELINE_DECK_PATH.to_string(),
            rewinds_before_restart: None,
            restart_strategy: RestartStrategy::Fresh,
            logging_mode: LoggingMode::Silent,
        }
    }
}

#[allow(dead_code)] // Infrastructure for future config flexibility
impl RewindPlayAgainConfig {
    /// Create a new config with the same deck for both players
    pub fn with_same_deck(deck_path: &str) -> Self {
        RewindPlayAgainConfig {
            deck1_path: deck_path.to_string(),
            deck2_path: deck_path.to_string(),
            ..Default::default()
        }
    }

    /// Set the rewind percentage
    pub fn rewind_percent(mut self, percent: f64) -> Self {
        self.rewind_percent = percent;
        self
    }

    /// Set the number of rewinds before restart
    pub fn rewinds_before_restart(mut self, rewinds: Option<usize>) -> Self {
        self.rewinds_before_restart = rewinds;
        self
    }

    /// Set the restart strategy
    pub fn restart_strategy(mut self, strategy: RestartStrategy) -> Self {
        self.restart_strategy = strategy;
        self
    }

    /// Set the logging mode
    pub fn logging_mode(mut self, mode: LoggingMode) -> Self {
        self.logging_mode = mode;
        self
    }
}

/// Trait for batch benchmark execution
///
/// Provides a unified interface for running batches of game simulations,
/// supporting both sequential and parallel execution strategies.
#[allow(dead_code)] // Infrastructure for future use
pub trait BatchBenchmark {
    /// Execute a batch of games
    ///
    /// # Parameters
    /// - `batch_size`: Number of games to execute
    /// - `num_threads`: Number of threads to use (implementations may restrict this)
    ///
    /// # Returns
    /// Duration of the batch execution
    ///
    /// # Errors
    /// Returns an error if the num_threads parameter is not supported by this implementation
    fn execute_batch(&self, batch_size: usize, num_threads: usize) -> Result<Duration, String>;

    /// Get aggregated metrics collected so far
    fn get_metrics(&self) -> GameMetrics;

    /// Get total number of games played
    fn total_games(&self) -> usize;

    /// Get the original seed used for this benchmark
    ///
    /// Returns the base RNG seed that was used when creating the benchmark instance.
    /// This can be used to derive per-thread seeds for parallel execution.
    fn orig_seed(&self) -> u64;

    /// Reseed the benchmark's RNG
    ///
    /// Updates the seed used for subsequent game executions. This is primarily
    /// used in parallel execution to ensure different threads explore different
    /// game paths rather than repeating the same games.
    ///
    /// # Parameters
    /// - `seed`: New seed value to use
    fn reseed(&mut self, seed: u64);

    /// Reset all accumulated metrics to zero
    ///
    /// Clears all metrics state, ensuring that subsequent calls to `get_metrics()`
    /// and `total_games()` only reflect measurements taken after the reset.
    /// This is used to ensure benchmarks only measure the last/biggest batch.
    fn reset_metrics(&self);

    /// Set the wall-clock duration (used by parallel executors)
    ///
    /// Overwrites the accumulated duration with the actual wall-clock time.
    /// In parallel execution, threads accumulate their individual durations (CPU time)
    /// into total_duration_nanos. This method overwrites that with the actual wall time
    /// while preserving CPU time in total_core_nanos.
    ///
    /// # Parameters
    /// - `duration`: The wall-clock duration to set
    fn set_wall_time(&self, duration: Duration);
}
