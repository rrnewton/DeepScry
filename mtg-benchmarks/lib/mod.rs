//! Benchmark library infrastructure
//!
//! This module provides reusable benchmark infrastructure including:
//! - Allocator configuration and tracking (`allocator`)
//! - Type definitions for metrics and traits (`types`)
//! - Utility functions for setup and helpers (`utils`)
//! - Core benchmark implementations (`core_benches`)
//! - Parallel execution utilities (`par_utils`)

#[path = "../benches/allocator.rs"]
pub mod allocator;
#[path = "../benches/core_benches.rs"]
pub mod core_benches;
pub mod par_utils;
pub mod types;
pub mod utils;

// Re-export commonly used items for convenience
// Note: Some re-exports are only used by certain binaries, so we allow unused
#[allow(unused_imports)]
pub use core_benches::RewindPlayAgain;
#[allow(unused_imports)]
pub use par_utils::{ParPinned, ParRayon};
#[allow(unused_imports)]
pub use types::{BatchBenchmark, GameMetrics, LoggingMode, RestartStrategy, RewindPlayAgainConfig, BASELINE_DECK_PATH};
#[allow(unused_imports)]
pub use utils::{
    ensure_correct_working_directory, get_benchmark_measurement_time, get_benchmark_num_threads,
    print_aggregated_metrics, BenchmarkSetup,
};
