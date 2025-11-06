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
// Note: Some re-exports are only used by certain binaries, so we allow unused
#[allow(unused_imports)]
pub use benches::{ParPinned, ParRayon, RewindPlayAgain};
#[allow(unused_imports)]
pub use types::{BatchBenchmark, GameMetrics, RestartStrategy, RewindPlayAgainConfig};
#[allow(unused_imports)]
pub use utils::{ensure_correct_working_directory, get_benchmark_measurement_time, BenchmarkSetup, BASELINE_DECK_PATH};
