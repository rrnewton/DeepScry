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

    // ========================================================================
    // Box<T, A> Verification Tests (mtg-154)
    // ========================================================================

    #[test]
    fn test_box_with_bump_allocator() {
        let bump = Bump::new();

        // Basic Box allocation with custom allocator
        let boxed = Box::new_in(42, &bump);
        assert_eq!(*boxed, 42);

        // Box with struct
        #[derive(Debug, PartialEq)]
        struct Data {
            value: i32,
            name: &'static str,
        }

        let boxed_struct = Box::new_in(
            Data {
                value: 100,
                name: "test",
            },
            &bump,
        );
        assert_eq!(boxed_struct.value, 100);
        assert_eq!(boxed_struct.name, "test");
    }

    #[test]
    fn test_box_trait_object_with_allocator() {
        let bump = Bump::new();

        // Box trait objects with custom allocator
        let trait_obj: Box<dyn std::fmt::Display, _> = Box::new_in(42, &bump);
        assert_eq!(format!("{}", trait_obj), "42");

        let trait_obj2: Box<dyn std::fmt::Display, _> = Box::new_in("hello", &bump);
        assert_eq!(format!("{}", trait_obj2), "hello");
    }

    #[test]
    fn test_box_error_with_allocator() {
        let bump = Bump::new();

        // Box<dyn Error> pattern common in error handling
        let err: Box<dyn std::fmt::Display, _> = Box::new_in("test error", &bump);
        assert_eq!(format!("{}", err), "test error");

        // Box with Debug trait
        let debug_box: Box<dyn std::fmt::Debug, _> = Box::new_in(vec![1, 2, 3], &bump);
        assert_eq!(format!("{:?}", debug_box), "[1, 2, 3]");
    }

    #[test]
    fn test_box_recursive_type_with_allocator() {
        let bump = Bump::new();

        // Recursive type (linked list) using Box<T, A>
        #[allow(dead_code)]
        enum List<A: std::alloc::Allocator> {
            Nil,
            Cons(i32, Box<List<A>, A>),
        }

        let list = List::Cons(1, Box::new_in(List::Cons(2, Box::new_in(List::Nil, &bump)), &bump));

        // Verify structure
        match list {
            List::Cons(val, next) => {
                assert_eq!(val, 1);
                match *next {
                    List::Cons(val2, ref next2) => {
                        assert_eq!(val2, 2);
                        assert!(matches!(**next2, List::Nil));
                    }
                    _ => panic!("Expected Cons"),
                }
            }
            _ => panic!("Expected Cons"),
        }
    }

    #[test]
    fn test_box_dyn_controller_pattern() {
        let bump = Bump::new();

        // Simulate the Controller trait pattern from the codebase
        trait Controller {
            fn choose_action(&self) -> i32;
        }

        struct RandomController {
            seed: u64,
        }

        impl Controller for RandomController {
            fn choose_action(&self) -> i32 {
                (self.seed % 10) as i32
            }
        }

        struct HeuristicController {
            depth: u32,
        }

        impl Controller for HeuristicController {
            fn choose_action(&self) -> i32 {
                self.depth as i32 * 2
            }
        }

        // Box<dyn Controller> with custom allocator
        let controller1: Box<dyn Controller, _> =
            Box::new_in(RandomController { seed: 42 }, &bump);
        assert_eq!(controller1.choose_action(), 2);

        let controller2: Box<dyn Controller, _> =
            Box::new_in(HeuristicController { depth: 5 }, &bump);
        assert_eq!(controller2.choose_action(), 10);
    }

    #[test]
    fn test_box_with_multiple_trait_objects() {
        let bump = Bump::new();

        // Vec of Box<dyn Trait> - common pattern in game engine
        let mut controllers: Vec<Box<dyn std::fmt::Display, _>, _> = Vec::new_in(&bump);

        controllers.push(Box::new_in(42, &bump));
        controllers.push(Box::new_in("test", &bump));
        controllers.push(Box::new_in(3.14, &bump));

        assert_eq!(format!("{}", controllers[0]), "42");
        assert_eq!(format!("{}", controllers[1]), "test");
        assert_eq!(format!("{}", controllers[2]), "3.14");
    }

    #[test]
    fn test_box_leak_with_allocator() {
        let bump = Bump::new();

        // Test Box::leak() behavior with custom allocator
        let boxed = Box::new_in(vec![1, 2, 3], &bump);
        let leaked: &mut Vec<i32> = Box::leak(boxed);

        leaked.push(4);
        assert_eq!(leaked.len(), 4);
        assert_eq!(leaked[3], 4);

        // Memory remains in bump arena until reset/drop
    }

    #[test]
    fn test_box_large_allocation() {
        let bump = Bump::new();

        // Large Box allocation
        let large_vec = vec![0u8; 10000];
        let boxed = Box::new_in(large_vec, &bump);

        assert_eq!(boxed.len(), 10000);
        assert_eq!(boxed[5000], 0);
    }

    #[test]
    fn test_box_nested_with_vec() {
        let bump = Bump::new();

        // Box containing Vec with same allocator
        let inner_vec = Vec::new_in(&bump);
        let boxed_vec: Box<Vec<i32, _>, _> = Box::new_in(inner_vec, &bump);

        // Both Box and Vec use the same bump allocator
        assert_eq!(boxed_vec.len(), 0);
    }
}
