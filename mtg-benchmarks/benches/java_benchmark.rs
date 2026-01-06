//! Criterion benchmark for Java Forge headless mode
//!
//! This benchmark invokes the Java headless simulator with increasing game counts
//! to measure per-game throughput while factoring out JVM startup overhead.
//!
//! Criterion's linear regression will separate:
//! - Fixed cost: JVM startup, card database loading, etc.
//! - Marginal cost: Time per additional game
//!
//! Run with: cargo bench --bench java_benchmark

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

/// Path to the Java headless script (relative to project root)
const HEADLESS_SCRIPT: &str = "forge-java/headless.sh";

/// Path to the test deck (relative to forge-java directory)
const TEST_DECK: &str = "forge-headless/test_decks/grizzly_bears.dck";

/// Find the project root directory by looking for Cargo.toml
fn find_project_root() -> Option<PathBuf> {
    let mut dir = std::env::current_dir().ok()?;
    loop {
        // Check if forge-java directory exists here
        if dir.join("forge-java").exists() {
            return Some(dir);
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// Run Java headless simulation and return elapsed time in seconds
fn run_java_simulation(num_games: u32, use_random_controller: bool) -> Option<Duration> {
    let project_root = find_project_root()?;

    let mut cmd = Command::new("bash");
    cmd.current_dir(&project_root)
        .arg(HEADLESS_SCRIPT)
        .arg("sim")
        .arg("-d")
        .arg(TEST_DECK)
        .arg("-d")
        .arg(TEST_DECK)
        .arg("-n")
        .arg(num_games.to_string())
        .arg("-q") // Quiet mode - only output results
        .arg("-c")
        .arg("30"); // 30 second timeout per game

    if use_random_controller {
        cmd.arg("-r");
    }

    let start = std::time::Instant::now();
    let output = cmd.output().ok()?;
    let elapsed = start.elapsed();

    if output.status.success() {
        Some(elapsed)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        eprintln!("Java simulation failed: {}", stderr);
        None
    }
}

/// Benchmark Java headless with random controller
/// Uses increasing game counts for linear regression
fn bench_java_random_controller(c: &mut Criterion) {
    let mut group = c.benchmark_group("java_headless");

    // Java is slow - use minimal samples to get quick results
    // Each Java run takes ~25-35s for startup + games
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(5));

    // Test with varying game counts to enable linear regression
    // Criterion will fit: time = startup_cost + (per_game_cost * num_games)
    // Using 1, 5, 20, 50 to get a good spread for regression
    for num_games in [1, 5, 20, 50].iter() {
        group.throughput(Throughput::Elements(u64::from(*num_games)));

        group.bench_with_input(
            BenchmarkId::new("random_controller", num_games),
            num_games,
            |b, &num_games| {
                b.iter_custom(|iters| {
                    let mut total = Duration::ZERO;
                    for _ in 0..iters {
                        if let Some(duration) = run_java_simulation(num_games, true) {
                            total += duration;
                        }
                    }
                    total
                });
            },
        );
    }

    group.finish();
}

/// Benchmark comparing Java AI vs Random controller
fn bench_java_ai_vs_random(c: &mut Criterion) {
    let mut group = c.benchmark_group("java_headless_comparison");

    group.measurement_time(Duration::from_secs(30));
    group.sample_size(5); // Fewer samples for slow AI mode

    let num_games = 1; // Just 1 game for comparison

    // Random controller
    group.bench_function("random_1game", |b| {
        b.iter_custom(|iters| {
            let mut total = Duration::ZERO;
            for _ in 0..iters {
                if let Some(duration) = run_java_simulation(num_games, true) {
                    total += duration;
                }
            }
            total
        });
    });

    // Full AI (much slower)
    group.bench_function("full_ai_1game", |b| {
        b.iter_custom(|iters| {
            let mut total = Duration::ZERO;
            for _ in 0..iters {
                if let Some(duration) = run_java_simulation(num_games, false) {
                    total += duration;
                }
            }
            total
        });
    });

    group.finish();
}

criterion_group!(benches, bench_java_random_controller, bench_java_ai_vs_random);
criterion_main!(benches);
