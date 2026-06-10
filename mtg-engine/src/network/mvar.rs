//! MVar: A blocking synchronization primitive for network choice coordination
//!
//! An MVar (mutable variable) is a synchronization primitive that supports blocking
//! `take` operations. Unlike IVars (which are write-once), MVars can be emptied and
//! refilled, making them suitable for streaming choice synchronization.
//!
//! This implementation uses a queue internally to buffer multiple values, allowing
//! the network thread to push choices faster than the game loop consumes them.
//!
//! ## Operations
//!
//! - `put(value)`: Add a value to the queue, never blocks
//! - `take()`: Remove and return the next value, blocks if queue is empty
//! - `try_take()`: Non-blocking take, returns None if empty
//!
//! ## Usage in Network Architecture
//!
//! ```text
//! WebSocket Reader                    Game Loop / Controllers
//!       │                                      │
//!       │ ─── put(ChoiceRequest) ──────────►   │
//!       │                                      │ ◄─ take() blocks
//!       │ ─── put(ChoiceRequest) ──────────►   │
//!       │                                      │ ◄─ take() returns
//! ```
//!
//! NOTE: this MVar carries our OWN `ChoiceRequest` delivery only
//! (`local_choice_mvar`). The OPPONENT's choices do NOT flow through an MVar —
//! they ride an append-only `ActionLog<ChoiceEntry>` cursor buffer
//! (`push_opponent_choice` / `take_opponent_choice`) so a rewind/replay can
//! reset the cursor and re-hand them in order (the log-as-source-of-truth
//! model). An earlier revision pushed `OpponentChoice` through this MVar; that
//! eager path is gone (mtg-786).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex};

/// A blocking queue with MVar-style take semantics
///
/// Supports multiple producers (network thread) and single consumer (game loop).
/// The `take` operation blocks until a value is available.
#[derive(Debug)]
pub struct MVar<T> {
    /// Queue of pending values
    queue: Mutex<VecDeque<T>>,
    /// Condition variable for signaling availability
    ready: Condvar,
    /// Exit flag to unblock waiting threads on shutdown
    exit_flag: AtomicBool,
}

impl<T> MVar<T> {
    /// Create a new empty MVar
    pub fn new() -> Self {
        Self {
            queue: Mutex::new(VecDeque::new()),
            ready: Condvar::new(),
            exit_flag: AtomicBool::new(false),
        }
    }

    /// Put a value into the MVar (non-blocking, queues if not empty)
    ///
    /// Unlike traditional MVars that block on put when full, this implementation
    /// queues values to handle network message bursts.
    pub fn put(&self, value: T) {
        let mut queue = self.queue.lock().unwrap();
        queue.push_back(value);
        self.ready.notify_one();
    }

    /// Take a value from the MVar (blocking)
    ///
    /// Blocks until a value is available, then removes and returns it.
    /// Returns `None` only if `signal_exit()` has been called and the queue is empty.
    pub fn take(&self) -> Option<T> {
        let mut queue = self.queue.lock().unwrap();

        // Wait for value to be available
        while queue.is_empty() {
            if self.exit_flag.load(Ordering::Relaxed) {
                return None;
            }
            queue = self.ready.wait(queue).unwrap();
        }

        queue.pop_front()
    }

    /// Try to take a value without blocking
    ///
    /// Returns `Some(value)` if available, `None` otherwise.
    #[allow(dead_code)]
    pub fn try_take(&self) -> Option<T> {
        let mut queue = self.queue.lock().unwrap();
        queue.pop_front()
    }

    /// Signal that consumers should exit
    ///
    /// Causes blocking `take()` calls to return `None` when the queue is empty.
    pub fn signal_exit(&self) {
        self.exit_flag.store(true, Ordering::Relaxed);
        self.ready.notify_all();
    }

    /// Check if exit has been signaled.
    ///
    /// Part of the generic MVar primitive's public API. As of the netarch
    /// dedicated-terminal-flag rework (mtg-629, phase 2 step 3),
    /// `SharedNetworkState::should_exit` reads the dedicated `terminal`
    /// flag instead of this, so this accessor currently has no in-tree caller
    /// outside the mvar unit tests.
    #[allow(dead_code)]
    pub fn is_exit_signaled(&self) -> bool {
        self.exit_flag.load(Ordering::Relaxed)
    }
}

impl<T> Default for MVar<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(clippy::redundant_clone)]
#[allow(clippy::clone_on_ref_ptr)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn test_put_take_single_value() {
        let mvar: MVar<i32> = MVar::new();
        mvar.put(42);
        assert_eq!(mvar.take(), Some(42));
    }

    #[test]
    fn test_put_take_multiple_values() {
        let mvar: MVar<i32> = MVar::new();
        mvar.put(1);
        mvar.put(2);
        mvar.put(3);
        assert_eq!(mvar.take(), Some(1));
        assert_eq!(mvar.take(), Some(2));
        assert_eq!(mvar.take(), Some(3));
    }

    #[test]
    fn test_try_take_empty() {
        let mvar: MVar<i32> = MVar::new();
        assert_eq!(mvar.try_take(), None);
    }

    #[test]
    fn test_signal_exit_unblocks_take() {
        let mvar: Arc<MVar<i32>> = Arc::new(MVar::new());
        let mvar_clone = mvar.clone();

        let handle = thread::spawn(move || {
            // This should block initially
            mvar_clone.take()
        });

        // Give the thread time to start blocking
        thread::sleep(Duration::from_millis(10));

        // Signal exit
        mvar.signal_exit();

        // The thread should now return None
        let result = handle.join().unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn test_concurrent_put_take() {
        let mvar: Arc<MVar<i32>> = Arc::new(MVar::new());
        let mvar_producer = mvar.clone();
        let mvar_consumer = mvar.clone();

        let producer = thread::spawn(move || {
            for i in 0..10 {
                mvar_producer.put(i);
            }
        });

        let consumer = thread::spawn(move || {
            let mut values = Vec::new();
            for _ in 0..10 {
                if let Some(v) = mvar_consumer.take() {
                    values.push(v);
                }
            }
            values
        });

        producer.join().unwrap();
        let values = consumer.join().unwrap();

        assert_eq!(values, (0..10).collect::<Vec<_>>());
    }
}
