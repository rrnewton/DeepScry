//! Benchmark library infrastructure
//!
//! This module provides reusable benchmark infrastructure including:
//! - Type definitions for metrics and traits (`types`)
//! - Utility functions for setup and helpers (`utils`)
//! - Core benchmark implementations (`benches`)

pub mod benches;
pub mod types;
pub mod utils;

// Re-export commonly used items for convenience
pub use benches::{ParRayon, RewindPlayAgain};
pub use types::{AtomicMetrics, BatchBenchmark, GameMetrics, GameOutcome};
pub use utils::{create_midgame_state, ensure_correct_working_directory, get_benchmark_measurement_time, BenchmarkSetup, BASELINE_DECK_PATH};
