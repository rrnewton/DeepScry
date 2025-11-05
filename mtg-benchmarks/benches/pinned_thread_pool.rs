//! Pinned thread pool for precise parallel benchmark timing
//!
//! This module implements a custom thread pool with:
//! - Thread pinning to physical CPU cores
//! - Spin barriers for precise synchronization (ready/go)
//! - Shared atomic counter for precise finish time tracking
//! - Main thread participates as worker 0
//!
//! This design provides more accurate timing than Rayon for parallel benchmarks
//! by eliminating thread scheduling variability and providing precise start/stop timing.

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

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
#[allow(dead_code)] // Will be used in future benchmarks
pub fn execute_parallel_batch<T, F, R>(num_threads: usize, template: &T, work_fn: F) -> (Duration, Vec<R>)
where
    T: Clone + Send + 'static,
    F: Fn(usize, &mut T) -> R + Send + Sync + 'static,
    R: Send + 'static,
{
    let core_ids = core_affinity::get_core_ids().expect("Failed to get core IDs");
    assert!(
        num_threads <= core_ids.len(),
        "Requested {} threads but only {} cores available",
        num_threads,
        core_ids.len()
    );

    // Pin main thread to core 0
    core_affinity::set_for_current(core_ids[0]);

    // Shared synchronization primitives
    let ready_flags: Arc<Vec<AtomicBool>> = Arc::new((0..num_threads).map(|_| AtomicBool::new(false)).collect());
    let go_flag = Arc::new(AtomicBool::new(false));
    let finish_counter = Arc::new(AtomicUsize::new(0));
    let finish_time_nanos = Arc::new(AtomicU64::new(0));

    // Shared work function
    let work_fn = Arc::new(work_fn);

    // Fork N-1 worker threads and collect their handles
    let mut worker_handles = Vec::new();

    for thread_id in 1..num_threads {
        let core_id = core_ids[thread_id];
        let mut thread_data = template.clone();
        let ready_flags_clone = Arc::clone(&ready_flags);
        let go_flag_clone = Arc::clone(&go_flag);
        let finish_counter_clone = Arc::clone(&finish_counter);
        let finish_time_clone = Arc::clone(&finish_time_nanos);
        let work_fn_clone = Arc::clone(&work_fn);

        let handle = thread::spawn(move || {
            // Pin this worker thread to its assigned core
            core_affinity::set_for_current(core_id);

            // Signal ready
            ready_flags_clone[thread_id].store(true, Ordering::Release);

            // Spin waiting for go signal
            while !go_flag_clone.load(Ordering::Acquire) {
                std::hint::spin_loop();
            }

            // Record local start time (for duration calculation if this thread finishes last)
            let local_start = Instant::now();

            // Execute work
            let result = work_fn_clone(thread_id, &mut thread_data);

            // Increment finish counter
            let count = finish_counter_clone.fetch_add(1, Ordering::AcqRel) + 1;

            // If this thread is the last to finish, record the time
            if count == num_threads {
                let duration = local_start.elapsed();
                finish_time_clone.store(duration.as_nanos() as u64, Ordering::Release);
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

    // Increment finish counter for main thread
    let count = finish_counter.fetch_add(1, Ordering::AcqRel) + 1;

    // If main thread is last to finish, record time
    if count == num_threads {
        let duration = start.elapsed();
        finish_time_nanos.store(duration.as_nanos() as u64, Ordering::Release);
    }

    // Spin waiting for finish time to be written
    while finish_time_nanos.load(Ordering::Acquire) == 0 {
        std::hint::spin_loop();
    }

    let duration_nanos = finish_time_nanos.load(Ordering::Acquire);
    let duration = Duration::from_nanos(duration_nanos);

    // Wait for all worker threads to exit and collect results
    let mut results = vec![main_result];
    for handle in worker_handles {
        results.push(handle.join().expect("Worker thread panicked"));
    }

    (duration, results)
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
    fn test_parallel_batch() {
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
