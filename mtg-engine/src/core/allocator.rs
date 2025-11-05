//! Custom allocator support for per-thread memory management
//!
//! This module provides utilities for using bump allocators with game state,
//! enabling per-thread allocation for parallel simulation with zero contention.
//!
//! ## Overview
//!
//! The standard Rust allocator (Global) uses locks that cause contention in parallel code.
//! By using per-thread bump allocators (`bumpalo::Bump`), we can:
//! - Eliminate allocator lock contention (zero shared state between threads)
//! - Achieve extremely fast allocation (bump pointer increment)
//! - Bulk deallocate when simulation completes (drop entire arena)
//!
//! ## Usage
//!
//! Thanks to bumpalo's `allocator_api` feature, `&Bump` already implements
//! `std::alloc::Allocator` and can be used directly with standard collections:
//!
//! ```rust,ignore
//! use bumpalo::Bump;
//!
//! // Create a bump arena for this thread
//! let bump = Bump::new();
//!
//! // Use &Bump directly as an allocator
//! let mut v = Vec::new_in(&bump);
//! v.push(1);
//! v.push(2);
//! v.push(3);
//!
//! // Create GameState using the bump allocator
//! let game = GameState::new_in(&bump);
//!
//! // Run simulation...
//! // All allocations go to the bump arena
//!
//! // Drop game and bump - bulk deallocation
//! ```
//!
//! ## Performance
//!
//! Target: Improve parallel efficiency from 5.6% to 50-60% (see mtg-a6ca26)
//!
//! Expected improvements:
//! - Eliminate 40% overhead from allocator contention
//! - Reduce cache coherency traffic
//! - Enable near-linear scaling on physical cores

/// Re-export bumpalo::Bump for convenience
pub use bumpalo::Bump;

/// Thread-local storage for simulation arenas.
///
/// This provides a convenient way to use per-thread bump allocators
/// without manual lifetime management.
///
/// # Example
///
/// ```rust,ignore
/// use mtg_forge_rs::core::allocator::with_simulation_arena;
///
/// let result = with_simulation_arena(|bump| {
///     let mut v = Vec::new_in(bump);
///     v.push(42);
///     v[0]
/// });
/// assert_eq!(result, 42);
/// ```
#[cfg(feature = "thread-local-arenas")]
thread_local! {
    static SIMULATION_ARENA: std::cell::RefCell<Bump> =
        std::cell::RefCell::new(Bump::new());
}

/// Run a closure with access to a thread-local bump allocator.
///
/// The arena is automatically reset before the closure runs, ensuring
/// a clean state for each simulation.
///
/// # Example
///
/// ```rust,ignore
/// let result = with_simulation_arena(|bump| {
///     let mut game = GameState::new_in(bump);
///     run_simulation(&mut game)
/// });
/// ```
///
/// # Safety
///
/// The allocator becomes invalid after the closure returns. Do not
/// store references to allocated memory beyond the closure's lifetime.
#[cfg(feature = "thread-local-arenas")]
#[inline]
pub fn with_simulation_arena<F, R>(f: F) -> R
where
    F: FnOnce(&Bump) -> R,
{
    SIMULATION_ARENA.with(|arena_cell| {
        let mut arena = arena_cell.borrow_mut();

        // Reset arena for clean state
        arena.reset();

        // Run closure with &Bump (which implements Allocator)
        f(&arena)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bump_with_vec() {
        let bump = Bump::new();

        // &Bump implements Allocator, so we can use it directly
        let mut v = Vec::new_in(&bump);
        v.push(1);
        v.push(2);
        v.push(3);

        assert_eq!(v.len(), 3);
        assert_eq!(v[0], 1);
        assert_eq!(v[1], 2);
        assert_eq!(v[2], 3);
    }

    #[test]
    fn test_bump_with_multiple_vecs() {
        let bump = Bump::new();

        // Multiple collections can share the same bump arena
        let mut v1 = Vec::new_in(&bump);
        let mut v2 = Vec::new_in(&bump);

        v1.push(10);
        v1.push(20);

        v2.push(30);
        v2.push(40);

        assert_eq!(v1.len(), 2);
        assert_eq!(v2.len(), 2);
        assert_eq!(v1[0], 10);
        assert_eq!(v2[0], 30);
    }

    #[test]
    fn test_bump_reset() {
        let mut bump = Bump::new();

        {
            let mut v = Vec::new_in(&bump);
            v.push(1);
            v.push(2);
            v.push(3);
            // v dropped here, but memory not freed yet
        }

        // Reset clears the arena
        bump.reset();

        // Can allocate again from clean state
        let mut v2 = Vec::new_in(&bump);
        v2.push(10);
        assert_eq!(v2.len(), 1);
        assert_eq!(v2[0], 10);
    }

    #[test]
    fn test_bump_large_allocation() {
        let bump = Bump::new();

        // Allocate a large vector
        let mut v = Vec::with_capacity_in(1000, &bump);
        for i in 0..1000 {
            v.push(i);
        }

        assert_eq!(v.len(), 1000);
        assert_eq!(v[500], 500);
    }

    #[test]
    #[cfg(feature = "thread-local-arenas")]
    fn test_with_simulation_arena() {
        let result = with_simulation_arena(|bump| {
            let mut v = Vec::new_in(bump);
            v.push(42);
            v[0]
        });

        assert_eq!(result, 42);

        // Running again should reset the arena
        let result2 = with_simulation_arena(|bump| {
            let v: Vec<i32, _> = Vec::new_in(bump);
            v.len()
        });

        assert_eq!(result2, 0);
    }

    #[test]
    #[cfg(feature = "thread-local-arenas")]
    fn test_with_simulation_arena_multiple_allocations() {
        with_simulation_arena(|bump| {
            let mut v1 = Vec::new_in(bump);
            let mut v2 = Vec::new_in(bump);

            v1.push(1);
            v1.push(2);

            v2.push(10);
            v2.push(20);
            v2.push(30);

            assert_eq!(v1.len(), 2);
            assert_eq!(v2.len(), 3);
        });
    }
}
