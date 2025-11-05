# Allocator API Implementation Plan

## Goal

Parameterize `GameState` and core collections with Rust's `Allocator` trait to enable:
1. Per-thread bump allocators for parallel simulations (zero contention)
2. Fast allocation (bump pointer, no locks)
3. Bulk deallocation when simulation completes
4. Target: 70-80% parallel efficiency (from current 5.6%)

## Background

### Current Problem (from mtg-a6ca26)

**Parallel efficiency: 5.6%** on 32 cores (should be 70-80%)
- Per-thread: 17.8x slower than sequential
- Root causes:
  1. Allocator contention: 40% of overhead (glibc malloc global locks)
  2. Memory system contention: 60% of overhead (cache misses, NUMA)

### Why Allocator API?

Using Rust's unstable `allocator_api` feature allows:
- Per-collection allocator selection (not just global)
- Each simulation thread gets its own bump allocator
- Zero contention between threads
- Extremely fast allocation (pointer increment vs malloc)
- Bulk deallocation (drop entire arena)

## Implementation Strategy

### Phase 1: Enable Allocator API Infrastructure

1. **Enable nightly feature in Cargo.toml**
   ```toml
   [features]
   allocator_api = []

   [dependencies]
   bumpalo = { version = "3.16", features = ["collections", "boxed"] }
   ```

2. **Create allocator wrapper module**
   - File: `mtg-engine/src/core/allocator_wrapper.rs`
   - Implement `Allocator` trait for `bumpalo::Bump`
   - Handle Global allocator as default
   - Type aliases for ease of use

### Phase 2: Parameterize Core Types

1. **GameState<A: Allocator = Global>**
   - Add allocator type parameter
   - Store allocator reference/instance
   - Pass to all owned collections

2. **Update all Vec/HashMap/SmallVec to use allocator**
   ```rust
   // Before:
   pub struct GameState {
       pub cards: Vec<Card>,
       pub zones: HashMap<Zone, Vec<CardId>>,
   }

   // After:
   pub struct GameState<A: Allocator = Global> {
       pub cards: Vec<Card, A>,
       pub zones: HashMap<Zone, Vec<CardId, A>, ..., A>,
       allocator: A,
   }
   ```

3. **Collections to parameterize**:
   - `GameState::cards: Vec<Card>`
   - `GameState::zones: HashMap<Zone, Vec<CardId>>`
   - `GameState::undo_log: UndoLog` (contains Vec internally)
   - `GameState::stack: Vec<StackEntry>`
   - `Player::hand/library/graveyard/exile: Vec<CardId>`
   - `ManaEngine` internal buffers
   - Any other Vec/HashMap in hot paths

### Phase 3: Bump Allocator Wrapper

1. **Create BumpAllocator wrapper**
   ```rust
   pub struct BumpAllocator<'a> {
       bump: &'a bumpalo::Bump,
   }

   unsafe impl<'a> Allocator for BumpAllocator<'a> {
       fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
           // Forward to bumpalo::Bump::alloc_layout
       }

       fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
           // No-op: bump allocators don't deallocate individual items
       }
   }
   ```

2. **Thread-local arena pattern**
   ```rust
   thread_local! {
       static SIMULATION_ARENA: RefCell<Bump> = RefCell::new(Bump::new());
   }

   pub fn with_simulation_allocator<F, R>(f: F) -> R
   where
       F: FnOnce(&BumpAllocator) -> R
   {
       SIMULATION_ARENA.with(|arena| {
           arena.borrow_mut().reset(); // Clear previous
           let alloc = BumpAllocator { bump: unsafe { &*arena.as_ptr() } };
           f(&alloc)
       })
   }
   ```

### Phase 4: Update Benchmark

1. **Modify parallel benchmark to use bump allocators**
   ```rust
   snapshots.into_par_iter()
       .for_each(|mut game| {
           with_simulation_allocator(|alloc| {
               // Clone game state using bump allocator
               let mut sim_game = game.clone_in(alloc);

               // Run simulation
               run_forward_gameplay_from_snapshot(&mut sim_game, seed);

               // sim_game dropped, but memory stays in bump
               // Bump reset at next iteration
           })
       });
   ```

2. **Compare performance**:
   - Baseline: Global allocator
   - Test 1: Per-thread bump allocator
   - Test 2: Disable undo logging + bump allocator
   - Measure parallel efficiency improvement

## Type System Challenges

### Challenge 1: Allocator Lifetime

**Problem**: Allocator has lifetime 'a, but GameState needs to own data

**Solution**: Use reference to allocator
```rust
pub struct GameState<'a, A: Allocator = &'a Global> {
    allocator: A,
    cards: Vec<Card, A>,
    // Lifetime 'a ties allocator to data
}
```

### Challenge 2: Clone with Different Allocator

**Problem**: `Clone` trait doesn't support changing allocator

**Solution**: Custom `clone_in()` method
```rust
impl<A: Allocator> GameState<A> {
    pub fn clone_in<'b, B: Allocator>(&self, alloc: &'b B) -> GameState<'b, B> {
        GameState {
            allocator: alloc,
            cards: self.cards.iter().cloned().collect_in(alloc),
            zones: self.zones.iter().map(|(k, v)| {
                (k.clone(), v.iter().copied().collect_in(alloc))
            }).collect_in(alloc),
            // ... etc
        }
    }
}
```

### Challenge 3: Backwards Compatibility

**Problem**: Existing code uses `GameState` without allocator

**Solution**: Default type parameter `A = Global`
```rust
// Old code still works:
let game = GameState::new(); // Uses Global allocator

// New code can specify:
let game = GameState::new_in(&bump_alloc);
```

## Implementation Order

1. **Week 1: Infrastructure**
   - Enable allocator_api feature
   - Create allocator_wrapper.rs module
   - Implement Allocator for Bump
   - Write basic tests

2. **Week 2: Core Types**
   - Parameterize GameState
   - Update all Vec/HashMap in GameState
   - Update Player, ManaEngine, etc.
   - Fix compilation errors

3. **Week 3: Clone and Helpers**
   - Implement clone_in() method
   - Add new_in() constructor
   - Helper functions for common patterns
   - Integration tests

4. **Week 4: Benchmark and Measure**
   - Update parallel benchmark
   - Run performance tests
   - Compare vs baseline
   - Document results

## Expected Outcomes

### Current State (2025-11-05)
- Parallel efficiency: 5.6%
- Aggregate speedup: 1.80x on 32 cores
- Per-thread: 17.8x slower than sequential

### After Bump Allocators (Predicted)
- Parallel efficiency: 50-60%
- Aggregate speedup: 16-19.2x on 32 cores
- Per-thread: 1.6-2.0x slower than sequential

### With All Optimizations (Target)
- Clone reduction + allocation reduction + bump allocators
- Parallel efficiency: 70-80%
- Aggregate speedup: 22.4-25.6x on 32 cores
- Aggregate throughput: 1.1M - 1.25M games/sec

## Risks and Mitigation

### Risk 1: Lifetime Complexity

**Risk**: Allocator lifetimes make API complex and hard to use

**Mitigation**:
- Keep default `A = Global` for simple cases
- Document common patterns clearly
- Provide helper functions to hide complexity

### Risk 2: Performance Regression

**Risk**: Allocator indirection adds overhead in sequential case

**Mitigation**:
- Benchmark sequential performance before/after
- Use zero-cost abstractions where possible
- Monomorphization should eliminate most overhead

### Risk 3: Incomplete Migration

**Risk**: Some hot paths still use Global allocator

**Mitigation**:
- Profile to find remaining allocations
- Systematic review of all Vec/HashMap usage
- Allocation tracking to verify reduction

## Related Issues

- **mtg-a6ca26**: Parallel MCTS optimization epic (parent issue)
- **mtg-61ea98**: GameState clone optimization (Option B is this work)
- **mtg-13**: Arena allocation for per-turn temporaries
- **mtg-2**: General optimization tracking

## References

- [Rust Allocator API (unstable)](https://doc.rust-lang.org/std/alloc/trait.Allocator.html)
- [bumpalo crate](https://docs.rs/bumpalo/)
- [Custom Allocators in Rust (blog)](https://nical.github.io/posts/rust-custom-allocators.html)
- [allocator-api2 (stable polyfill)](https://docs.rs/allocator-api2/)

## Success Criteria

1. ✅ GameState compiles with allocator parameter
2. ✅ Existing code continues to work (default Global)
3. ✅ Per-thread bump allocators work in parallel benchmark
4. ✅ Parallel efficiency improves to >50%
5. ✅ No sequential performance regression
6. ✅ All tests pass
7. ✅ Documentation updated

---

**Status**: Planning complete, ready to begin implementation
**Owner**: Claude / AI
**Target**: Implement in allocator feature branch
