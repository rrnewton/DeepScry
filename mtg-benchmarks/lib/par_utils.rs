//! Parallel execution utilities for batch benchmarks
//!
//! This module provides:
//! - `execute_parallel_batch`: Low-level pinned thread pool with spin barriers
//! - `ParRayon<T>`: High-level wrapper using Rayon's thread pool
//! - `ParPinned<T>`: High-level wrapper using pinned threads for microsecond-accurate timing
//!
//! Both wrappers are generic over any `BatchBenchmark` implementation and follow
//! the same pattern: clone the inner benchmark for each thread, distribute work
//! evenly with quotient/remainder, and aggregate results.

use super::types::{BatchBenchmark, GameMetrics};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// Seed spacing between threads to prevent overlap
///
/// Each thread's seed is derived as: `orig_seed + (thread_id * THREAD_SEED_SPACING)`
/// This ensures that when the inner sequential loop increments the seed for each game,
/// the seeds from different threads don't overlap.
///
/// With 1,000,000 spacing, each thread can execute up to 1 million games before
/// potentially overlapping with the next thread's seed space.
const THREAD_SEED_SPACING: u64 = 1_000_000;

/// Derive a per-thread seed from the original seed and thread ID
///
/// # Parameters
/// - `orig_seed`: The base seed used for the benchmark
/// - `thread_id`: The thread's index (0, 1, 2, ...)
///
/// # Returns
/// A seed value that is spaced far enough from other threads to prevent overlap
/// when the sequential loop increments seeds for individual games.
///
/// # Example
/// ```ignore
/// let thread_0_seed = derive_thread_seed(43, 0); // 43 + 0*1000000 = 43
/// let thread_1_seed = derive_thread_seed(43, 1); // 43 + 1*1000000 = 1000043
/// let thread_2_seed = derive_thread_seed(43, 2); // 43 + 2*1000000 = 2000043
/// ```
#[inline]
fn derive_thread_seed(orig_seed: u64, thread_id: usize) -> u64 {
    orig_seed.wrapping_add((thread_id as u64).wrapping_mul(THREAD_SEED_SPACING))
}

/// Helper function to run parallel batch with pinned threads and precise timing
///
/// This is a simplified interface that:
/// 1. Clones input data for each worker thread
/// 2. Runs work function on each thread's data
/// 3. Returns precise timing from last thread to finish
///
/// # Type Parameters
/// - `T`: Type of data to clone for each worker (must be Clone + Send)
/// - `F`: Work function that takes (thread_id, &mut T) and returns result R
/// - `R`: Result type from work function (must be Send)
///
/// # Parameters
/// - `num_threads`: Number of threads to use
/// - `template`: Data to clone for each worker thread
/// - `work_fn`: Function to execute on each thread
///
/// # Returns
/// Tuple of (Duration, Vec<R>) where Duration is precise batch time and Vec contains
/// results from each thread
#[allow(dead_code)] // Used by benchmarks but not by all binaries
fn execute_parallel_batch<T, F, R>(num_threads: usize, template: &T, work_fn: F) -> (Duration, Vec<R>)
where
    T: Clone + Send + 'static,
    F: Fn(usize, &mut T) -> R + Send + Sync + 'static,
    R: Send + 'static,
{
    let core_ids = core_affinity::get_core_ids().expect("Failed to get core IDs");

    // Check if we have enough cores for pinning
    // If not enough cores available (e.g., in containers), skip pinning but continue with parallel execution
    let enable_pinning = num_threads <= core_ids.len();

    if !enable_pinning {
        eprintln!(
            "Warning: Requested {} threads but only {} cores available via core_affinity",
            num_threads,
            core_ids.len()
        );
        eprintln!("         Skipping thread pinning, but continuing with parallel execution");
    }

    // Save original affinity so we can restore it later
    // Note: We get the affinity by calling get_core_ids() which returns cores we CAN run on
    let original_affinity_core_count = core_ids.len();
    let original_cpu_count = num_cpus::get(); // Save before pinning!

    // Pin main thread to core 0 if pinning is enabled
    if enable_pinning {
        core_affinity::set_for_current(core_ids[0]);
    }

    // Shared synchronization primitives
    let ready_flags: Arc<Vec<AtomicBool>> = Arc::new((0..num_threads).map(|_| AtomicBool::new(false)).collect());
    let go_flag = Arc::new(AtomicBool::new(false));
    let finish_counter = Arc::new(AtomicUsize::new(0));
    // Store finish instant - the last thread to finish records its Instant
    // Main thread will calculate duration from its start time
    let finish_instant: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));

    // Shared work function
    let work_fn = Arc::new(work_fn);

    // Fork N-1 worker threads and collect their handles
    let mut worker_handles = Vec::new();

    for thread_id in 1..num_threads {
        let core_id = if enable_pinning {
            Some(core_ids[thread_id])
        } else {
            None
        };
        let mut thread_data = template.clone();
        let ready_flags_clone = Arc::clone(&ready_flags);
        let go_flag_clone = Arc::clone(&go_flag);
        let finish_counter_clone = Arc::clone(&finish_counter);
        let finish_instant_clone = Arc::clone(&finish_instant);
        let work_fn_clone = Arc::clone(&work_fn);

        let handle = thread::spawn(move || {
            // Pin this worker thread to its assigned core (if pinning is enabled)
            if let Some(core) = core_id {
                core_affinity::set_for_current(core);
            }

            // Signal ready
            ready_flags_clone[thread_id].store(true, Ordering::Release);

            // Spin waiting for go signal
            while !go_flag_clone.load(Ordering::Acquire) {
                std::hint::spin_loop();
            }

            // Execute work
            let result = work_fn_clone(thread_id, &mut thread_data);

            // Record finish instant
            let now = Instant::now();

            // Increment finish counter
            let count = finish_counter_clone.fetch_add(1, Ordering::AcqRel) + 1;

            // If this thread is the last to finish, record the finish instant
            // Main thread will calculate the duration from its start time
            if count == num_threads {
                *finish_instant_clone.lock().unwrap() = Some(now);
            }

            result
        });

        worker_handles.push(handle);
    }

    // Wait for all worker threads to be ready
    for (_tid, flag) in ready_flags.iter().enumerate().skip(1) {
        while !flag.load(Ordering::Acquire) {
            std::hint::spin_loop();
        }
    }

    // Record start time and signal go
    let start = Instant::now();
    go_flag.store(true, Ordering::Release);

    // Main thread executes work as worker 0
    let mut main_data = template.clone();
    let main_result = work_fn(0, &mut main_data);

    // Record main thread's finish instant
    let main_finish = Instant::now();

    // Increment finish counter for main thread
    let count = finish_counter.fetch_add(1, Ordering::AcqRel) + 1;

    // If main thread is last to finish, record its finish instant
    if count == num_threads {
        *finish_instant.lock().unwrap() = Some(main_finish);
    }

    // Wait for the last thread to record its finish instant
    let final_instant = loop {
        if let Some(instant) = *finish_instant.lock().unwrap() {
            break instant;
        }
        std::hint::spin_loop();
    };

    // Calculate duration from the main thread's start time to the last thread's finish time
    let duration = final_instant.duration_since(start);

    // Wait for all worker threads to exit and collect results
    let mut results = vec![main_result];
    for handle in worker_handles {
        results.push(handle.join().expect("Worker thread panicked"));
    }

    // Restore original affinity by unpinning (allowing all cores again)
    // We do this by attempting to set affinity to all available cores
    // Unfortunately core_affinity doesn't have an "unpin" API, so we need to reset to all cores
    if enable_pinning && original_affinity_core_count > 1 {
        // Use taskset to restore full affinity to all originally available cores
        // Use the saved original_cpu_count, not num_cpus::get() which will return 1 while pinned
        let pid = std::process::id();
        let cpu_range = format!("0-{}", original_cpu_count - 1);
        let _ = std::process::Command::new("taskset")
            .args(["-cp", &cpu_range, &pid.to_string()])
            .output();
    }

    (duration, results)
}

/// Parallel wrapper using Rayon for batch benchmark execution
///
/// Wraps any BatchBenchmark implementation and provides parallel execution
/// using Rayon's thread pool. The wrapped benchmark must support sequential
/// execution (num_threads=1).
///
/// # Example
/// ```ignore
/// let sequential_bench = RewindPlayAgain::new(config, "SEQUENTIAL");
/// let parallel_bench = ParRayon::new(sequential_bench);
/// parallel_bench.execute_batch(1000, 8)?; // Run 1000 games on 8 threads
/// ```
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

impl<T: BatchBenchmark + Clone + Send> BatchBenchmark for ParRayon<T> {
    fn execute_batch(&self, batch_size: usize, num_threads: usize) -> Result<Duration, String> {
        use rayon::prelude::*;

        if num_threads < 1 {
            return Err(format!("num_threads must be >= 1, got {}", num_threads));
        }

        // For single-threaded execution, just delegate to the inner benchmark
        if num_threads == 1 {
            return self.inner.execute_batch(batch_size, 1);
        }

        // Calculate iterations per thread: quotient for all, remainder goes to thread 0
        let iters_per_thread = batch_size / num_threads;
        let remainder = batch_size % num_threads;

        // Get original seed for deriving per-thread seeds
        let orig_seed = self.inner.orig_seed();

        // Start timing
        let start = std::time::Instant::now();

        // PER-BATCH SETUP (outside timing): Clone snapshots for parallel execution
        // We need to clone before the parallel loop because GameState contains RefCell (not Sync)

        // Box up the clones to allow trait objects and avoid Sized issues
        // Each thread gets a clone reseeded with proper spacing to prevent overlap
        let replicas: Vec<Box<T>> = (0..num_threads)
            .map(|thread_id| {
                let mut clone = self.inner.clone();
                // Reseed with spaced seeds to ensure different game paths per thread
                // Spacing prevents overlap when sequential loop increments seed per game
                clone.reseed(derive_thread_seed(orig_seed, thread_id));
                Box::new(clone)
            })
            .collect();

        // Execute in parallel using Rayon
        // Each thread calls the inner benchmark's sequential execute_batch
        replicas.into_par_iter().enumerate().try_for_each(
            |(thread_id, local_self): (usize, Box<T>)| -> Result<(), String> {
                // Thread 0 gets the quotient plus the remainder
                let thread_iters = if thread_id == 0 {
                    iters_per_thread + remainder
                } else {
                    iters_per_thread
                };

                if thread_iters > 0 {
                    local_self.execute_batch(thread_iters, 1)?;
                }
                Ok(())
            },
        )?;

        // Get wall-clock duration
        let wall_time = start.elapsed();

        // Overwrite accumulated CPU time with actual wall time
        // (Each thread added its duration to total_duration_nanos, giving CPU time;
        //  we now overwrite with wall time while preserving CPU time in total_core_nanos)
        self.inner.set_wall_time(wall_time);

        Ok(wall_time)
    }

    fn get_metrics(&self) -> GameMetrics {
        self.inner.get_metrics()
    }

    fn total_games(&self) -> usize {
        self.inner.total_games()
    }

    fn orig_seed(&self) -> u64 {
        self.inner.orig_seed()
    }

    fn reseed(&mut self, seed: u64) {
        self.inner.reseed(seed);
    }

    fn reset_metrics(&self) {
        self.inner.reset_metrics();
    }

    fn set_wall_time(&self, duration: Duration) {
        self.inner.set_wall_time(duration);
    }
}

/// Parallel wrapper using pinned threads for batch benchmark execution
///
/// Wraps any BatchBenchmark implementation and provides parallel execution
/// using pinned threads with spin barriers for microsecond-accurate timing.
/// The wrapped benchmark must support sequential execution (num_threads=1).
///
/// This provides more accurate timing than ParRayon by:
/// - Pinning threads to physical CPU cores
/// - Using spin barriers for synchronization (no OS scheduler involvement)
/// - Recording precise start/finish times with atomic counters
///
/// # Example
/// ```ignore
/// let sequential_bench = RewindPlayAgain::new(config, "SEQUENTIAL");
/// let parallel_bench = ParPinned::new(sequential_bench);
/// parallel_bench.execute_batch(1000, 8)?; // Run 1000 games on 8 pinned threads
/// ```
#[allow(dead_code)] // Infrastructure for future use
pub struct ParPinned<T> {
    inner: T,
}

#[allow(dead_code)] // Infrastructure for future use
impl<T> ParPinned<T> {
    /// Create a new pinned-parallel wrapper around a sequential benchmark
    pub fn new(inner: T) -> Self {
        ParPinned { inner }
    }

    /// Get a reference to the inner benchmark
    pub fn inner(&self) -> &T {
        &self.inner
    }
}

impl<T: BatchBenchmark + Clone + Send + 'static> BatchBenchmark for ParPinned<T> {
    fn execute_batch(&self, batch_size: usize, num_threads: usize) -> Result<Duration, String> {
        if num_threads < 1 {
            return Err(format!("num_threads must be >= 1, got {}", num_threads));
        }

        // For single-threaded execution, just delegate to the inner benchmark
        if num_threads == 1 {
            return self.inner.execute_batch(batch_size, 1);
        }

        // Calculate iterations per thread: quotient for all, remainder goes to thread 0
        let iters_per_thread = batch_size / num_threads;
        let remainder = batch_size % num_threads;

        // Get original seed for deriving per-thread seeds
        let orig_seed = self.inner.orig_seed();

        // Execute parallel batch with pinned threads
        // Each thread gets a clone of the inner benchmark and executes a portion
        let (batch_duration, results) =
            execute_parallel_batch(num_threads, &self.inner, move |thread_id, local_self| {
                // Reseed with spaced seeds to ensure different game paths per thread
                // Spacing prevents overlap when sequential loop increments seed per game
                local_self.reseed(derive_thread_seed(orig_seed, thread_id));

                // Thread 0 gets the quotient plus the remainder
                let thread_iters = if thread_id == 0 {
                    iters_per_thread + remainder
                } else {
                    iters_per_thread
                };

                if thread_iters > 0 {
                    local_self.execute_batch(thread_iters, 1)
                } else {
                    Ok(Duration::ZERO)
                }
            });

        // All threads should succeed or we propagate the error
        for result in results {
            result?;
        }

        // Overwrite accumulated CPU time with actual wall time
        // (Each thread added its duration to total_duration_nanos, giving CPU time;
        //  we now overwrite with wall time while preserving CPU time in total_core_nanos)
        self.inner.set_wall_time(batch_duration);

        Ok(batch_duration)
    }

    fn get_metrics(&self) -> GameMetrics {
        self.inner.get_metrics()
    }

    fn total_games(&self) -> usize {
        self.inner.total_games()
    }

    fn orig_seed(&self) -> u64 {
        self.inner.orig_seed()
    }

    fn reseed(&mut self, seed: u64) {
        self.inner.reseed(seed);
    }

    fn reset_metrics(&self) {
        self.inner.reset_metrics();
    }

    fn set_wall_time(&self, duration: Duration) {
        self.inner.set_wall_time(duration);
    }
}

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use super::execute_parallel_batch;
    #[allow(unused_imports)]
    use std::sync::atomic::{AtomicUsize, Ordering};
    #[allow(unused_imports)]
    use std::sync::Arc;

    #[test]
    fn test_execute_parallel_batch() {
        let num_threads = num_cpus::get_physical().min(4);
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = Arc::clone(&counter);

        let (duration, results) = execute_parallel_batch(
            num_threads,
            &42u32, // template value
            move |thread_id, data| {
                counter_clone.fetch_add(1, Ordering::Relaxed);
                *data + thread_id as u32
            },
        );

        // All threads should have incremented the counter
        assert_eq!(counter.load(Ordering::Relaxed), num_threads);

        // Should have result from each thread
        assert_eq!(results.len(), num_threads);

        // Duration should be non-zero
        assert!(duration.as_nanos() > 0);
    }
}
