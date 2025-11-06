//! Standalone binary for running rewind + play again benchmark
//!
//! This bypasses Criterion and runs a single batch directly, printing metrics.
//!
//! Usage:
//!   cargo run --release --package mtg-benchmarks --bin rewind_bench [batch_size]
//!
//! Default batch size: 1000 games

// Import allocator for stats tracking - we define it here, not in allocator module
use stats_alloc::{Region, StatsAlloc, INSTRUMENTED_SYSTEM};
use std::alloc::System;

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

// Include benchmark allocator types (but not the global allocator)
mod allocator {
    pub use stats_alloc::{Region, INSTRUMENTED_SYSTEM};
    use std::alloc::System;

    /// Allocation statistics - works with or without tracking
    #[derive(Debug, Clone, Copy, Default)]
    #[allow(dead_code)]
    pub struct AllocStats {
        pub bytes_allocated: usize,
        pub bytes_deallocated: usize,
        pub bytes_reallocated: usize,
        pub allocations: usize,
        pub deallocations: usize,
        pub reallocations: usize,
    }

    impl AllocStats {
        pub const fn zero() -> Self {
            AllocStats {
                bytes_allocated: 0,
                bytes_deallocated: 0,
                bytes_reallocated: 0,
                allocations: 0,
                deallocations: 0,
                reallocations: 0,
            }
        }
    }

    impl From<stats_alloc::Stats> for AllocStats {
        fn from(stats: stats_alloc::Stats) -> Self {
            AllocStats {
                bytes_allocated: stats.bytes_allocated,
                bytes_deallocated: stats.bytes_deallocated,
                bytes_reallocated: stats.bytes_reallocated.max(0) as usize,
                allocations: stats.allocations,
                deallocations: stats.deallocations,
                reallocations: stats.reallocations,
            }
        }
    }

    pub struct AllocTracker {
        region: Region<'static, System>,
    }

    impl AllocTracker {
        pub fn new() -> Self {
            AllocTracker {
                region: Region::new(&INSTRUMENTED_SYSTEM),
            }
        }

        pub fn stats(&self) -> AllocStats {
            self.region.change().into()
        }
    }
}

#[path = "../../benches/pinned_thread_pool.rs"]
#[allow(dead_code)]
mod pinned_thread_pool;

// Include the benchmark library
#[path = "../../benches/lib/mod.rs"]
mod benchlib;

use benchlib::RewindPlayAgain;

fn main() {
    // Parse batch size from command line (default 1000)
    let batch_size = std::env::args()
        .nth(1)
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(1000);

    println!("=== Rewind + Play Again Benchmark ===");
    println!("Batch size: {} games", batch_size);
    println!();

    // Create benchmark instance (loads deck and creates midgame state)
    println!("Initializing benchmark...");
    let benchmark = RewindPlayAgain::new("SEQUENTIAL");
    let seed = benchmark.seed();
    println!("  Seed: {}", seed);
    println!();

    // Execute batch
    println!("Executing batch of {} games...", batch_size);
    let region = Region::new(GLOBAL);
    let batch_duration = benchmark.execute_batch_sequential(batch_size);
    let stats = region.change();

    // Get aggregated metrics
    let metrics = benchmark.get_aggregated_metrics();
    let total_games = benchmark.get_total_games();

    // Print results
    println!();
    println!("=== Results ===");
    println!("Total games: {}", total_games);
    println!("Total duration: {:.3}s", batch_duration.as_secs_f64());
    println!(
        "Avg duration/game: {:.3}ms",
        batch_duration.as_secs_f64() * 1000.0 / total_games as f64
    );
    println!();

    println!("=== Game Metrics ===");
    println!("Total turns: {}", metrics.turns);
    println!("Total actions: {}", metrics.actions);
    println!("Avg turns/game: {:.2}", metrics.turns as f64 / total_games as f64);
    println!("Avg actions/game: {:.2}", metrics.actions as f64 / total_games as f64);
    println!("Actions/turn: {:.2}", metrics.actions_per_turn());
    println!();

    println!("=== Allocation Metrics ===");
    println!("Total bytes allocated: {}", stats.bytes_allocated);
    println!("Total bytes deallocated: {}", stats.bytes_deallocated);
    println!(
        "Net bytes: {}",
        stats.bytes_allocated as i64 - stats.bytes_deallocated as i64
    );
    println!(
        "Avg bytes/game: {:.2}",
        stats.bytes_allocated as f64 / total_games as f64
    );
    println!("Bytes/turn: {:.2}", metrics.bytes_per_turn());
    println!();

    // Print win rates
    benchmark.print_win_rates("SEQUENTIAL");
}
