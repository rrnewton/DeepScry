//! Custom allocator support for per-thread memory management
//!
//! This module provides wrapper types for using custom allocators with game state,
//! enabling per-thread bump allocators for parallel simulation with zero contention.
//!
//! ## Overview
//!
//! The standard Rust allocator (Global) uses locks that cause contention in parallel code.
//! By using per-thread bump allocators, we can:
//! - Eliminate allocator lock contention (zero shared state)
//! - Achieve extremely fast allocation (bump pointer increment)
//! - Bulk deallocate when simulation completes (drop entire arena)
//!
//! ## Usage
//!
//! ```rust,ignore
//! use mtg_forge_rs::core::allocator::BumpAllocator;
//! use bumpalo::Bump;
//!
//! // Create a bump arena for this thread
//! let bump = Bump::new();
//! let alloc = BumpAllocator::new(&bump);
//!
//! // Create GameState using the bump allocator
//! let game = GameState::new_in(&alloc);
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

use std::alloc::{AllocError, Allocator, Layout};
use std::ptr::NonNull;

/// Wrapper around `bumpalo::Bump` that implements the `Allocator` trait.
///
/// This allows using bump allocation with standard library collections
/// like `Vec<T, A>` and `HashMap<K, V, S, A>`.
///
/// ## Safety
///
/// The bump allocator never deallocates individual allocations - memory
/// is only freed when the entire `Bump` arena is dropped. This is perfect
/// for per-simulation allocation patterns where all memory can be discarded
/// after the simulation completes.
#[derive(Debug)]
pub struct BumpAllocator<'a> {
    bump: &'a bumpalo::Bump,
}

impl<'a> BumpAllocator<'a> {
    /// Create a new allocator wrapping the given bump arena.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let bump = Bump::new();
    /// let alloc = BumpAllocator::new(&bump);
    /// ```
    #[inline]
    pub const fn new(bump: &'a bumpalo::Bump) -> Self {
        Self { bump }
    }

    /// Get a reference to the underlying bump arena.
    #[inline]
    pub const fn bump(&self) -> &'a bumpalo::Bump {
        self.bump
    }
}

impl<'a> Clone for BumpAllocator<'a> {
    #[inline]
    fn clone(&self) -> Self {
        // Cloning just copies the reference to the same bump arena
        // Multiple collections can share the same arena
        Self { bump: self.bump }
    }
}

impl<'a> Copy for BumpAllocator<'a> {}

unsafe impl<'a> Allocator for BumpAllocator<'a> {
    #[inline]
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        // Allocate from the bump arena
        let ptr = self.bump.alloc_layout(layout);

        // Convert raw pointer to NonNull<[u8]>
        // SAFETY: bumpalo guarantees non-null allocation or panic
        let slice = unsafe {
            std::slice::from_raw_parts_mut(ptr.as_ptr(), layout.size())
        };

        Ok(NonNull::from(slice))
    }

    #[inline]
    unsafe fn deallocate(&self, _ptr: NonNull<u8>, _layout: Layout) {
        // Bump allocators don't support individual deallocation
        // Memory is freed when the entire Bump is dropped or reset
        // This is a no-op
    }

    #[inline]
    fn allocate_zeroed(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        // Allocate and zero-initialize
        let ptr = self.allocate(layout)?;

        // SAFETY: We just allocated this memory, so it's valid for writes
        unsafe {
            std::ptr::write_bytes(ptr.as_mut_ptr(), 0, layout.size());
        }

        Ok(ptr)
    }
}

/// Thread-local storage for simulation arenas.
///
/// This provides a convenient way to use per-thread bump allocators
/// without manual lifetime management.
///
/// # Example
///
/// ```rust,ignore
/// with_simulation_arena(|alloc| {
///     let game = GameState::new_in(alloc);
///     // Run simulation...
///     // game dropped automatically, arena reset for next simulation
/// });
/// ```
#[cfg(feature = "thread-local-arenas")]
thread_local! {
    static SIMULATION_ARENA: std::cell::RefCell<bumpalo::Bump> =
        std::cell::RefCell::new(bumpalo::Bump::new());
}

/// Run a closure with access to a thread-local bump allocator.
///
/// The arena is automatically reset before the closure runs, ensuring
/// a clean state for each simulation.
///
/// # Example
///
/// ```rust,ignore
/// let result = with_simulation_arena(|alloc| {
///     let mut game = GameState::new_in(alloc);
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
    F: FnOnce(&BumpAllocator) -> R,
{
    SIMULATION_ARENA.with(|arena_cell| {
        let mut arena = arena_cell.borrow_mut();

        // Reset arena for clean state
        arena.reset();

        // Create allocator wrapper
        let alloc = BumpAllocator::new(&arena);

        // Run closure
        f(&alloc)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::alloc::Global;

    #[test]
    fn test_bump_allocator_basic() {
        let bump = bumpalo::Bump::new();
        let alloc = BumpAllocator::new(&bump);

        // Allocate a small buffer
        let layout = Layout::from_size_align(64, 8).unwrap();
        let result = alloc.allocate(layout);
        assert!(result.is_ok());

        let ptr = result.unwrap();
        assert_eq!(ptr.len(), 64);
        assert!(ptr.as_ptr() as usize % 8 == 0); // Check alignment
    }

    #[test]
    fn test_bump_allocator_zeroed() {
        let bump = bumpalo::Bump::new();
        let alloc = BumpAllocator::new(&bump);

        // Allocate zeroed buffer
        let layout = Layout::from_size_align(32, 4).unwrap();
        let ptr = alloc.allocate_zeroed(layout).unwrap();

        // Verify it's zeroed
        let slice = unsafe {
            std::slice::from_raw_parts(ptr.as_ptr() as *const u8, 32)
        };

        assert!(slice.iter().all(|&b| b == 0));
    }

    #[test]
    fn test_bump_allocator_multiple_allocations() {
        let bump = bumpalo::Bump::new();
        let alloc = BumpAllocator::new(&bump);

        // Multiple allocations should all succeed
        for _ in 0..100 {
            let layout = Layout::from_size_align(16, 8).unwrap();
            assert!(alloc.allocate(layout).is_ok());
        }
    }

    #[test]
    fn test_bump_allocator_with_vec() {
        let bump = bumpalo::Bump::new();
        let alloc = BumpAllocator::new(&bump);

        // Create a Vec using the bump allocator
        let mut v = Vec::new_in(alloc);
        v.push(1);
        v.push(2);
        v.push(3);

        assert_eq!(v.len(), 3);
        assert_eq!(v[0], 1);
        assert_eq!(v[1], 2);
        assert_eq!(v[2], 3);
    }

    #[test]
    fn test_deallocate_is_noop() {
        let bump = bumpalo::Bump::new();
        let alloc = BumpAllocator::new(&bump);

        let layout = Layout::from_size_align(64, 8).unwrap();
        let ptr = alloc.allocate(layout).unwrap();

        // Deallocate should not crash (it's a no-op)
        unsafe {
            alloc.deallocate(ptr.cast(), layout);
        }

        // Should still be able to allocate
        assert!(alloc.allocate(layout).is_ok());
    }

    #[test]
    fn test_allocator_is_copy() {
        let bump = bumpalo::Bump::new();
        let alloc1 = BumpAllocator::new(&bump);
        let alloc2 = alloc1; // Should copy, not move

        // Both should work
        let layout = Layout::from_size_align(32, 4).unwrap();
        assert!(alloc1.allocate(layout).is_ok());
        assert!(alloc2.allocate(layout).is_ok());
    }

    #[test]
    #[cfg(feature = "thread-local-arenas")]
    fn test_with_simulation_arena() {
        let result = with_simulation_arena(|alloc| {
            let mut v = Vec::new_in(*alloc);
            v.push(42);
            v[0]
        });

        assert_eq!(result, 42);

        // Running again should reset the arena
        let result2 = with_simulation_arena(|alloc| {
            let v: Vec<i32, _> = Vec::new_in(*alloc);
            v.len()
        });

        assert_eq!(result2, 0);
    }
}
