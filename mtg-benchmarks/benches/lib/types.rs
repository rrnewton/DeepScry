//! Type definitions for benchmark infrastructure
//!
//! This module defines core types and traits used across the benchmark suite,
//! including metrics tracking, batch execution, and game outcomes.

use crate::allocator::AllocStats;
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

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

/// Atomic version of GameMetrics for thread-safe aggregation
///
/// Mirrors GameMetrics but uses atomic types for concurrent updates.
/// Wrapped in Arc for cheap cloning across threads.
pub struct AtomicMetrics {
    /// Total games played
    pub total_games: AtomicUsize,
    /// Aggregated turns across all games
    pub total_turns: AtomicU32,
    /// Aggregated actions across all games
    pub total_actions: AtomicUsize,
    /// Aggregated duration in nanoseconds
    pub total_duration_nanos: AtomicU64,
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
            total_bytes_allocated: AtomicUsize::new(0),
            total_bytes_deallocated: AtomicUsize::new(0),
        }
    }

    /// Atomically add metrics from a batch
    pub fn add_batch(&self, games: usize, turns: u32, actions: usize, duration: Duration, alloc_stats: &AllocStats) {
        self.total_games.fetch_add(games, Ordering::Relaxed);
        self.total_turns.fetch_add(turns, Ordering::Relaxed);
        self.total_actions.fetch_add(actions, Ordering::Relaxed);
        self.total_duration_nanos
            .fetch_add(duration.as_nanos() as u64, Ordering::Relaxed);
        self.total_bytes_allocated
            .fetch_add(alloc_stats.bytes_allocated, Ordering::Relaxed);
        self.total_bytes_deallocated
            .fetch_add(alloc_stats.bytes_deallocated, Ordering::Relaxed);
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
}
