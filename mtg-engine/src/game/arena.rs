//! Game-local arena allocator
//!
//! Provides a per-game allocator that can reduce allocator contention in parallel
//! game simulations. Each game gets its own arena, eliminating cross-thread
//! contention on the global allocator.
//!
//! ## Design
//!
//! The `GameArena` can operate in two modes:
//! - **Global mode**: Uses the global allocator (default, for single-threaded use)
//! - **Bump mode**: Uses a bumpalo arena (for parallel MCTS simulations)
//!
//! In normal forward play, the arena is not reset until game end.
//! For MCTS rollouts, the arena can be reset between rollouts.
//!
//! ## Usage
//!
//! ```ignore
//! // Default: uses global allocator
//! let mut game = GameState::new_two_player(...);
//!
//! // For parallel simulations: use bump allocator
//! let mut game = GameState::new_two_player_with_arena(..., ArenaMode::Bump);
//! ```

use bumpalo::Bump;
use std::cell::RefCell;

/// Mode for the game arena allocator
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ArenaMode {
    /// Use the global allocator (default)
    /// Allocations go through standard Rust allocation
    #[default]
    Global,
    /// Use a per-game bump allocator
    /// Reduces contention in parallel simulations
    Bump,
}

/// Per-game arena allocator
///
/// Wraps either the global allocator or a bumpalo arena.
/// This struct is stored inside GameState and provides allocation
/// services for temporary game data.
///
/// Not serializable - will be recreated fresh when loading from snapshot.
pub struct GameArena {
    mode: ArenaMode,
    /// The bump arena (only used in Bump mode)
    /// Wrapped in RefCell for interior mutability (bumpalo requires &self for alloc)
    bump: RefCell<Option<Bump>>,
}

impl GameArena {
    /// Create a new arena in global mode (uses system allocator)
    pub fn new() -> Self {
        Self {
            mode: ArenaMode::Global,
            bump: RefCell::new(None),
        }
    }

    /// Create a new arena with the specified mode
    pub fn with_mode(mode: ArenaMode) -> Self {
        let bump = match mode {
            ArenaMode::Global => None,
            ArenaMode::Bump => Some(Bump::new()),
        };
        Self {
            mode,
            bump: RefCell::new(bump),
        }
    }

    /// Create a new arena in bump mode with pre-allocated capacity
    ///
    /// The capacity is in bytes. A typical game might use 50-100KB.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            mode: ArenaMode::Bump,
            bump: RefCell::new(Some(Bump::with_capacity(capacity))),
        }
    }

    /// Get the current arena mode
    #[inline]
    pub fn mode(&self) -> ArenaMode {
        self.mode
    }

    /// Check if this arena uses bump allocation
    #[inline]
    pub fn is_bump(&self) -> bool {
        self.mode == ArenaMode::Bump
    }

    /// Reset the bump arena, freeing all allocations
    ///
    /// This is a no-op in Global mode.
    /// In Bump mode, this resets the arena pointer, making all
    /// previous allocations invalid. Use with care!
    ///
    /// Typical usage: call at the end of a game or MCTS rollout.
    pub fn reset(&self) {
        if let Some(ref mut bump) = *self.bump.borrow_mut() {
            bump.reset();
        }
    }

    /// Get the allocated bytes in the bump arena
    ///
    /// Returns 0 in Global mode.
    pub fn allocated_bytes(&self) -> usize {
        self.bump.borrow().as_ref().map(|b| b.allocated_bytes()).unwrap_or(0)
    }

    /// Allocate a Vec with the given capacity
    ///
    /// In Global mode, this just creates a normal Vec.
    /// In Bump mode, this allocates from the bump arena.
    ///
    /// Note: The returned Vec still uses the global allocator for its
    /// backing storage. True bump-allocated collections require using
    /// bumpalo::collections::Vec directly. This method is a stepping stone
    /// for gradual migration.
    #[inline]
    pub fn alloc_vec<T>(&self, capacity: usize) -> Vec<T> {
        // For now, always use global allocator
        // TODO: In Bump mode, we could use bumpalo::collections::Vec
        // but that would require changing return types and lifetimes
        Vec::with_capacity(capacity)
    }

    /// Allocate a Vec and initialize with values from an iterator
    ///
    /// This is a convenience method that allocates and fills in one step.
    #[inline]
    pub fn alloc_vec_from_iter<T, I: IntoIterator<Item = T>>(&self, iter: I) -> Vec<T> {
        // For now, always use global allocator
        iter.into_iter().collect()
    }

    /// Get a reference to the underlying bump allocator (if in Bump mode)
    ///
    /// This is useful for code that wants to use bumpalo directly
    /// for more advanced allocation patterns.
    ///
    /// Returns None in Global mode.
    pub fn bump(&self) -> Option<std::cell::Ref<'_, Bump>> {
        let borrowed = self.bump.borrow();
        if borrowed.is_some() {
            Some(std::cell::Ref::map(borrowed, |opt| opt.as_ref().unwrap()))
        } else {
            None
        }
    }
}

impl Default for GameArena {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for GameArena {
    fn clone(&self) -> Self {
        // When cloning, create a fresh arena of the same mode
        // Don't clone the allocations - each clone gets its own arena
        Self::with_mode(self.mode)
    }
}

impl std::fmt::Debug for GameArena {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GameArena")
            .field("mode", &self.mode)
            .field("allocated_bytes", &self.allocated_bytes())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_global_mode() {
        let arena = GameArena::new();
        assert_eq!(arena.mode(), ArenaMode::Global);
        assert!(!arena.is_bump());
        assert_eq!(arena.allocated_bytes(), 0);

        // Vec allocation works
        let v: Vec<i32> = arena.alloc_vec(10);
        assert_eq!(v.capacity(), 10);
    }

    #[test]
    fn test_bump_mode() {
        let arena = GameArena::with_mode(ArenaMode::Bump);
        assert_eq!(arena.mode(), ArenaMode::Bump);
        assert!(arena.is_bump());

        // Bump starts empty (no allocation until first alloc)
        // Just verify we can get the allocated_bytes() without panic
        let _initial_bytes = arena.allocated_bytes();
    }

    #[test]
    fn test_bump_with_capacity() {
        let arena = GameArena::with_capacity(1024);
        assert!(arena.is_bump());
        // Capacity is a hint, actual allocation may differ
    }

    #[test]
    fn test_reset() {
        let arena = GameArena::with_mode(ArenaMode::Bump);

        // Do some allocations via the bump directly
        if let Some(bump) = arena.bump() {
            let _ = bump.alloc(42u64);
            let _ = bump.alloc([0u8; 100]);
        }

        let bytes_before = arena.allocated_bytes();
        assert!(bytes_before > 0);

        arena.reset();

        // After reset, bumpalo keeps allocated_bytes() the same (it's the chunk capacity)
        // but the arena is ready for reuse. New allocations will overwrite old data.
        // This is expected behavior - reset() doesn't deallocate memory, it reuses it.
        // The key property is that new allocations work correctly after reset.
        {
            let bump = arena.bump().unwrap();
            // Allocate something to verify the arena is usable after reset
            let val = bump.alloc(123u32);
            assert_eq!(*val, 123);
        }
    }

    #[test]
    fn test_clone() {
        let arena = GameArena::with_mode(ArenaMode::Bump);

        // Do some allocations
        if let Some(bump) = arena.bump() {
            let _ = bump.alloc([0u8; 100]);
        }

        // Clone should have same mode but fresh arena
        let cloned = arena.clone();
        assert_eq!(cloned.mode(), ArenaMode::Bump);
        assert_eq!(cloned.allocated_bytes(), 0);
    }

    #[test]
    fn test_global_reset_noop() {
        let arena = GameArena::new();
        // Reset should be a no-op in global mode
        arena.reset();
        assert_eq!(arena.allocated_bytes(), 0);
    }
}
