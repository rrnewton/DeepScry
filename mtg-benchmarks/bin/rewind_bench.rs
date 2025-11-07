//! Standalone binary for running rewind + play again benchmark
//!
//! This bypasses Criterion and runs a single batch directly, printing metrics.
//! Provides comprehensive CLI flags to control all benchmark configuration options.
//!
//! Usage:
//!   cargo run --release --package mtg-benchmarks --bin rewind_bench -- [OPTIONS]
//!
//! Examples:
//!   # Default settings (sequential, 1000 games, robots mirror)
//!   cargo run --release --bin rewind_bench
//!
//!   # Parallel execution with 8 threads
//!   cargo run --release --bin rewind_bench -- --mode par --threads 8 --batch-size 5000
//!
//!   # Custom decks with different settings
//!   cargo run --release --bin rewind_bench -- \
//!     --deck1 decks/old_school/05_mono_black_rogerbrand.dck \
//!     --deck2 decks/old_school/02_thedeck_peterschnidrig.dck \
//!     --rewind-percent 0.3 \
//!     --logging stdout
//!
//!   # Pinned thread execution for microsecond-accurate timing
//!   cargo run --release --bin rewind_bench -- --mode pinned --threads 16
//!
//!   # DHAT heap profiling (1000 games, robots mirror)
//!   cargo run --release --bin rewind_bench -- --dhat
//!   # View results with: dhat/dh_view.html (opens dhat-heap.json)

// Include the benchmark library FIRST so we don't conflict with its allocator
#[path = "../lib/mod.rs"]
mod benchlib;

use benchlib::{
    allocator::{allocator_name, GLOBAL},
    BatchBenchmark, FakePar, LoggingMode, ParPinned, ParRayon, RestartStrategy, RewindPlayAgain,
    RewindPlayAgainConfig, BASELINE_DECK_PATH,
};
use clap::Parser;
use stats_alloc::Region;

#[cfg(feature = "dhat-heap")]
use dhat::Profiler;

/// Execution mode for the benchmark
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExecutionMode {
    /// Sequential execution (single thread)
    Sequential,
    /// Fake parallel execution (sequential with parallel RNG seeding)
    FakePar,
    /// Parallel execution using Rayon thread pool
    Par,
    /// Parallel execution using pinned threads with core affinity
    Pinned,
}

impl std::str::FromStr for ExecutionMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "sequential" | "seq" => Ok(ExecutionMode::Sequential),
            "fakepar" | "fake-par" | "fake" => Ok(ExecutionMode::FakePar),
            "par" | "parallel" | "rayon" => Ok(ExecutionMode::Par),
            "pinned" | "pinned-par" => Ok(ExecutionMode::Pinned),
            _ => Err(format!("Invalid mode '{}'. Valid options: sequential, fakepar, par, pinned", s)),
        }
    }
}

/// Standalone benchmark runner for rewind + play again workload
#[derive(Parser, Debug)]
#[command(name = "rewind_bench")]
#[command(about = "Run rewind + play again benchmark without Criterion overhead", long_about = None)]
struct Args {
    /// Number of games to execute in the batch
    #[arg(short = 'n', long, default_value = "1000")]
    batch_size: usize,

    /// Execution mode: sequential, fakepar, par, or pinned
    #[arg(short = 'm', long, default_value = "sequential")]
    mode: ExecutionMode,

    /// Number of threads to use (only for fakepar/par/pinned modes)
    #[arg(short = 't', long)]
    threads: Option<usize>,

    /// Path to player 1's deck
    #[arg(long, default_value = BASELINE_DECK_PATH)]
    deck1: String,

    /// Path to player 2's deck
    #[arg(long, default_value = BASELINE_DECK_PATH)]
    deck2: String,

    /// Percentage of game to play before rewinding (0.0 to 1.0)
    #[arg(short = 'r', long, default_value = "0.5")]
    rewind_percent: f64,

    /// Number of rewind+replay rounds before restarting from scratch
    /// (None = infinite, 0 = play forward only)
    #[arg(long)]
    rewinds_before_restart: Option<usize>,

    /// Restart strategy: fresh or clone
    #[arg(long, default_value = "fresh")]
    restart_strategy: String,

    /// Logging mode: silent, memory, or stdout
    #[arg(short = 'l', long, default_value = "silent")]
    logging: String,

    /// Enable DHAT heap profiling (outputs dhat-heap.json)
    /// Standard workload: 1000 games, robots mirror, sequential mode
    #[arg(long)]
    dhat: bool,
}

impl Args {
    /// Convert CLI args to RewindPlayAgainConfig
    fn to_config(&self) -> Result<RewindPlayAgainConfig, String> {
        // Validate rewind_percent
        if !(0.0..=1.0).contains(&self.rewind_percent) {
            return Err(format!(
                "rewind-percent must be between 0.0 and 1.0, got {}",
                self.rewind_percent
            ));
        }

        // Parse restart strategy
        let restart_strategy = match self.restart_strategy.to_lowercase().as_str() {
            "fresh" => RestartStrategy::Fresh,
            "clone" => RestartStrategy::Clone,
            _ => {
                return Err(format!(
                    "Invalid restart strategy '{}'. Valid options: fresh, clone",
                    self.restart_strategy
                ))
            }
        };

        // Parse logging mode
        let logging_mode = match self.logging.to_lowercase().as_str() {
            "silent" => LoggingMode::Silent,
            "memory" | "mem" => LoggingMode::ToMemory,
            "stdout" | "out" => LoggingMode::ToStdout,
            _ => {
                return Err(format!(
                    "Invalid logging mode '{}'. Valid options: silent, memory, stdout",
                    self.logging
                ))
            }
        };

        Ok(RewindPlayAgainConfig {
            rewind_percent: self.rewind_percent,
            deck1_path: self.deck1.clone(),
            deck2_path: self.deck2.clone(),
            rewinds_before_restart: self.rewinds_before_restart,
            restart_strategy,
            logging_mode,
        })
    }

    /// Get the number of threads to use
    fn get_num_threads(&self) -> usize {
        match self.threads {
            Some(n) => n,
            None => {
                // Default thread count based on mode
                match self.mode {
                    ExecutionMode::Sequential => 1,
                    ExecutionMode::FakePar | ExecutionMode::Par | ExecutionMode::Pinned => num_cpus::get(),
                }
            }
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    // Initialize DHAT if requested
    #[cfg(feature = "dhat-heap")]
    let _profiler = if args.dhat {
        Some(Profiler::new_heap())
    } else {
        None
    };

    #[cfg(not(feature = "dhat-heap"))]
    if args.dhat {
        eprintln!("ERROR: --dhat flag requires rebuilding with --features dhat-heap");
        eprintln!("Run: cargo run --release --features dhat-heap --bin rewind_bench -- --dhat");
        std::process::exit(1);
    }

    // Validate and convert to config
    let config = args.to_config()?;
    let num_threads = args.get_num_threads();

    // Print configuration
    println!("=== Rewind + Play Again Benchmark ===");
    println!("Execution mode: {:?}", args.mode);
    println!("Batch size: {} games", args.batch_size);
    if args.mode != ExecutionMode::Sequential {
        println!("Threads: {}", num_threads);
    }
    if args.dhat {
        println!("DHAT Profiling: ENABLED (dhat-heap.json will be created)");
    }
    println!();

    println!("Configuration:");
    println!("  Deck 1: {}", config.deck1_path);
    println!("  Deck 2: {}", config.deck2_path);
    println!("  Rewind percent: {:.1}%", config.rewind_percent * 100.0);
    println!(
        "  Rewinds before restart: {}",
        match config.rewinds_before_restart {
            None => "infinite".to_string(),
            Some(0) => "0 (forward only)".to_string(),
            Some(n) => n.to_string(),
        }
    );
    println!("  Restart strategy: {:?}", config.restart_strategy);
    println!("  Logging mode: {:?}", config.logging_mode);
    println!("  Allocator: {}", allocator_name());
    println!();

    // Execute benchmark based on mode
    match args.mode {
        ExecutionMode::Sequential => run_sequential(config, args.batch_size),
        ExecutionMode::FakePar => run_fakepar(config, args.batch_size, num_threads),
        ExecutionMode::Par => run_parallel_rayon(config, args.batch_size, num_threads),
        ExecutionMode::Pinned => run_parallel_pinned(config, args.batch_size, num_threads),
    }
}

/// Run benchmark in sequential mode
fn run_sequential(config: RewindPlayAgainConfig, batch_size: usize) -> Result<(), Box<dyn std::error::Error>> {
    println!("Initializing sequential benchmark...");
    let benchmark = RewindPlayAgain::new(config, "SEQUENTIAL");
    let seed = benchmark.orig_seed();
    println!("  Seed: {}", seed);
    println!();

    // Execute batch with allocation tracking
    println!("Executing batch of {} games...", batch_size);
    let region = Region::new(GLOBAL);
    let batch_duration = benchmark.execute_batch_sequential(batch_size);
    let stats = region.change();

    // Get aggregated metrics
    let metrics = benchmark.get_aggregated_metrics();
    let total_games = benchmark.get_total_games();

    // Print results
    print_results(&metrics, total_games, batch_duration.as_secs_f64(), &stats);
    benchmark.print_win_rates("SEQUENTIAL");

    Ok(())
}

/// Run benchmark in fake-parallel mode (sequential with parallel RNG seeding)
fn run_fakepar(
    config: RewindPlayAgainConfig,
    batch_size: usize,
    num_threads: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("Initializing fake-parallel benchmark...");
    let base_benchmark = RewindPlayAgain::new(config, "FAKE-PARALLEL");
    let benchmark = FakePar::new(base_benchmark);
    let seed = benchmark.inner().orig_seed();
    println!("  Seed: {}", seed);
    println!();

    // Execute batch with allocation tracking
    println!(
        "Executing batch of {} games (sequential with {} logical threads)...",
        batch_size, num_threads
    );
    let region = Region::new(GLOBAL);
    let batch_duration = benchmark.execute_batch(batch_size, num_threads)?;
    let stats = region.change();

    // Get aggregated metrics
    let metrics = benchmark.get_metrics();
    let total_games = benchmark.total_games();

    // Print results
    print_results(&metrics, total_games, batch_duration.as_secs_f64(), &stats);
    benchmark.inner().print_win_rates("FAKE-PARALLEL");

    Ok(())
}

/// Run benchmark in parallel mode using Rayon
fn run_parallel_rayon(
    config: RewindPlayAgainConfig,
    batch_size: usize,
    num_threads: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("Initializing parallel benchmark (Rayon)...");
    let base_benchmark = RewindPlayAgain::new(config, "PARALLEL");
    let benchmark = ParRayon::new(base_benchmark);
    let seed = benchmark.inner().orig_seed();
    println!("  Seed: {}", seed);
    println!();

    // Execute batch with allocation tracking
    println!(
        "Executing batch of {} games across {} threads...",
        batch_size, num_threads
    );
    let region = Region::new(GLOBAL);
    let batch_duration = benchmark.execute_batch(batch_size, num_threads)?;
    let stats = region.change();

    // Get aggregated metrics
    let metrics = benchmark.get_metrics();
    let total_games = benchmark.total_games();

    // Print results
    print_results(&metrics, total_games, batch_duration.as_secs_f64(), &stats);
    benchmark.inner().print_win_rates("PARALLEL (Rayon)");

    Ok(())
}

/// Run benchmark in parallel mode using pinned threads
fn run_parallel_pinned(
    config: RewindPlayAgainConfig,
    batch_size: usize,
    num_threads: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("Initializing parallel benchmark (Pinned Threads)...");
    let base_benchmark = RewindPlayAgain::new(config, "PINNED-PARALLEL");
    let benchmark = ParPinned::new(base_benchmark);
    let seed = benchmark.inner().orig_seed();
    println!("  Seed: {}", seed);
    println!();

    // Execute batch with allocation tracking
    println!(
        "Executing batch of {} games across {} pinned threads...",
        batch_size, num_threads
    );
    let region = Region::new(GLOBAL);
    let batch_duration = benchmark.execute_batch(batch_size, num_threads)?;
    let stats = region.change();

    // Get aggregated metrics
    let metrics = benchmark.get_metrics();
    let total_games = benchmark.total_games();

    // Print results
    print_results(&metrics, total_games, batch_duration.as_secs_f64(), &stats);
    benchmark.inner().print_win_rates("PARALLEL (Pinned Threads)");

    Ok(())
}

/// Print benchmark results
fn print_results(metrics: &benchlib::GameMetrics, total_games: usize, duration_secs: f64, stats: &stats_alloc::Stats) {
    println!();
    println!("=== Results ===");
    println!("Total games: {}", total_games);
    println!("Total duration: {:.3}s", duration_secs);
    println!(
        "Avg duration/game: {:.3}ms",
        duration_secs * 1000.0 / total_games as f64
    );
    println!();

    println!("=== Game Metrics ===");
    println!("Total turns: {}", metrics.turns);
    println!("Total actions: {}", metrics.actions);
    println!("Avg turns/game: {:.2}", metrics.turns as f64 / total_games as f64);
    println!("Avg actions/game: {:.2}", metrics.actions as f64 / total_games as f64);
    println!("Actions/turn: {:.2}", metrics.actions_per_turn());
    println!("Games/sec: {:.2}", total_games as f64 / duration_secs);
    println!("Actions/sec: {:.2}", metrics.actions as f64 / duration_secs);
    println!("Turns/sec: {:.2}", metrics.turns as f64 / duration_secs);
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
    println!(
        "Bytes/action: {:.2}",
        if metrics.actions > 0 {
            stats.bytes_allocated as f64 / metrics.actions as f64
        } else {
            0.0
        }
    );
    println!();
}
