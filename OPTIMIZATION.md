# Performance Optimization Guide

This document provides guidance on high-performance Rust patterns for the MTG Forge project, with a focus on zero-copy patterns and minimizing allocations.

## Current Performance Metrics

### KEY TRACKING METRIC: robots_mirror/mem_logging_rewind_play_again

This is our primary optimization target metric (as of 2025-11-25_#901(efd29a7)):

- **Actions/sec**: 2,684,892 (2.68M/sec)
- **Bytes/action**: 228.59 bytes
- **Games/sec**: 325.71
- **Time/game**: 3.09 ms
- **Avg bytes/game**: 1.88 MB

**Note:** This benchmark uses the rewind+replay pattern with memory logging enabled, which isolates forward gameplay performance from initialization overhead. It's the standard metric for tracking optimization progress.

## Zero-Copy Patterns and Best Practices

### 1. Avoid Unnecessary `clone()`

**Problem**: Cloning creates deep copies of data, which is expensive for large structures.

**Solution**: Use references and manage lifetimes appropriately.

```rust
// ❌ BAD: Unnecessary clone
fn process_cards(cards: &Vec<Card>) -> Vec<Card> {
    cards.clone()
}

// ✅ GOOD: Return reference or iterator
fn process_cards(cards: &Vec<Card>) -> &[Card] {
    cards.as_slice()
}

// ✅ EVEN BETTER: Return iterator for lazy evaluation
fn process_cards(cards: &Vec<Card>) -> impl Iterator<Item = &Card> {
    cards.iter().filter(|c| c.is_creature())
}
```

**When to use `.iter().cloned()` vs `.clone().iter()`**:
- `v.iter().cloned()` creates a borrowed iterator that clones items on-the-fly (no Vec allocation)
- `v.clone().iter()` clones the entire Vec first (expensive heap allocation)
- Always prefer `v.iter().cloned()` when you need owned values from iteration

### 2. Avoid Unnecessary `collect()`

**Problem**: Calling `collect()` allocates a new collection when the data might only be iterated once.

**Solution**: Return iterator types (`impl Iterator<Item=T>`) instead of `Vec<T>`.

```rust
// ❌ BAD: Unnecessary collect
fn get_creatures(cards: &[Card]) -> Vec<&Card> {
    cards.iter().filter(|c| c.is_creature()).collect()
}

// ✅ GOOD: Return iterator
fn get_creatures(cards: &[Card]) -> impl Iterator<Item = &Card> + '_ {
    cards.iter().filter(|c| c.is_creature())
}
```

### 3. Chain Iterator Operations

**Problem**: Multiple `collect()` calls between operations create temporary collections.

**Solution**: Chain iterator methods together for a single traversal.

```rust
// ❌ BAD: Multiple collects
let creatures: Vec<_> = cards.iter().filter(|c| c.is_creature()).collect();
let tapped: Vec<_> = creatures.iter().filter(|c| c.is_tapped()).collect();

// ✅ GOOD: Chained operations
let tapped_creatures = cards.iter()
    .filter(|c| c.is_creature())
    .filter(|c| c.is_tapped());
```

### 4. Use Slices Instead of Owned Types

**Problem**: Taking owned `String` or `Vec<T>` when you only need to read.

**Solution**: Use `&str` instead of `&String`, and `&[T]` instead of `&Vec<T>`.

```rust
// ❌ BAD: Unnecessary specificity
fn print_name(name: &String) { }
fn process_cards(cards: &Vec<Card>) { }

// ✅ GOOD: Use slices
fn print_name(name: &str) { }
fn process_cards(cards: &[Card]) { }
```

### 5. Implement `size_hint()` for Custom Iterators

**Problem**: Collections can't pre-allocate if they don't know the iterator size.

**Solution**: Implement `Iterator::size_hint()` or `ExactSizeIterator::len()` when possible.

```rust
impl Iterator for MyIterator {
    type Item = Card;

    fn next(&mut self) -> Option<Self::Item> { /* ... */ }

    // Helps collect() and extend() pre-allocate
    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.remaining_count();
        (remaining, Some(remaining))
    }
}
```

### 6. Arena Allocation for Short-Lived Objects

**Problem**: Frequent small allocations and deallocations fragment memory and slow down the allocator.

**Solution**: Use arena allocators (like `bumpalo` or `typed-arena`) for per-frame or per-turn allocations.

```rust
// Consider for future optimization:
// - Per-turn arena for temporary combat calculations
// - Per-phase arena for stack resolution
// - Arena reset at phase/turn boundaries
```

**Benefits**:
- Allocation is just pointer increment (extremely fast)
- Deallocation is bulk operation (drop entire arena)
- Better cache locality (adjacent allocations)
- Particularly good for game engines with frame-based allocation patterns

### 7. Object Pools for Reusable Objects

**Problem**: Creating and destroying the same types of objects repeatedly (e.g., spell effects, combat damage calculations).

**Solution**: Pre-allocate a pool and reuse objects.

```rust
// Future consideration for:
// - Token pools
// - Effect pools
// - Combat calculation buffers
```

### 8. Use `SmallVec` and `SmallMap` for Expected-Small Collections

**Problem**: Many game entities have 0-2 counters/abilities but we allocate on the heap for any collection.

**Solution**: Use `smallvec::SmallVec` and similar crates to avoid heap allocation for small counts.

```rust
use smallvec::SmallVec;

// Stores up to 4 items inline, only heap-allocates if more
type CounterList = SmallVec<[Counter; 4]>;
```

**Already in use**: The project already uses `SmallVec` for counters (see PROJECT_VISION.md).

### 9. Prefer Unboxed Enums Over `Vec<Box<dyn Trait>>`

**Problem**: Java-style polymorphism with vectors of boxed trait objects creates pointer chasing and heap fragmentation.

**Solution**: Use enums with data variants when the set of types is closed.

```rust
// ❌ Less efficient: Boxed trait objects
Vec<Box<dyn Effect>>

// ✅ More efficient: Unboxed enum
enum Effect {
    DealDamage { target: EntityId, amount: u32 },
    DrawCards { player: PlayerId, count: u32 },
    // ... more variants
}
```

**Rust advantage**: Vectors of enums are stored contiguously without pointer indirection, unlike Java's object arrays.

### 10. Cow (Clone-on-Write) for Conditional Ownership

**Problem**: Sometimes you need owned data, sometimes borrowed, leading to unnecessary clones.

**Solution**: Use `std::borrow::Cow` to defer cloning until necessary.

```rust
use std::borrow::Cow;

fn process_name(name: Cow<str>) -> Cow<str> {
    if name.contains("transform") {
        Cow::Owned(name.to_uppercase()) // Only clone if needed
    } else {
        name // Return borrowed if no modification
    }
}
```

## Profiling and Measurement

### Running Benchmarks

```bash
# Run all benchmarks (slow)
make full-benchmark

# Track performance over time (automated)
./scripts/periodically_run_benchmarks.sh

# Or use cargo bench directly for specific tests:
cargo bench --bench game_benchmark -- robots_mirror/mem_logging_rewind_play_again
```

**Key metrics to track:**
- **Actions/sec** and **Bytes/action** from `robots_mirror/mem_logging_rewind_play_again` (primary metric)
- **Games/sec** and **Turns/sec** (throughput)
- **Bytes/turn** and **Bytes/game** (allocation pressure)

### Profiling Tools Available

All CPU profiling methods use **release/optimized builds** for accurate results.

#### CPU Profiling

##### 1. Callgrind Profiling (RECOMMENDED for CPU in containers)

```bash
make callgrindprofile
```

**Best for**: Detailed CPU instruction counting and call graph analysis
- **Uses**: `--release` build via `build-release` target
- **Works in**: Containers (no special permissions needed)
- **Output**: `experiment_results/callgrind.out`
- **Performance**: 250 games (~88s due to ~50x slowdown from instrumentation)
- **View with**:
  - `callgrind_annotate experiment_results/callgrind.out | less`
  - `kcachegrind experiment_results/callgrind.out` (GUI, requires X11)

**Advantages:**
- Works in containerized environments
- Shows CPU instruction counts (not wall-clock time affected by system load)
- Detailed call graph and cache simulation
- Source-level annotation

##### 2. Perf Profiling (Requires host/privileges)

```bash
make perfprofile
```

**Best for**: CPU hotspots and cache behavior on host systems
- **Uses**: `--release` build via `build-release` target
- **Works in**: Host systems with kernel permissions (NOT containers/WSL)
- **Output**: `experiment_results/perf.data`
- **Performance**: 5000 games for statistical significance
- **View with**: `cd experiment_results && sudo perf report`

**Note**: Will fail with "Operation not permitted" in containers - use `callgrindprofile` instead.

##### 3. Flamegraph Profiling (Requires host/privileges)

```bash
make profile
```

**Best for**: Visual flame graph of CPU time distribution
- **Uses**: `release` profile (confirmed in build output)
- **Works in**: Host systems with kernel permissions (NOT containers/WSL)
- **Output**: `experiment_results/flamegraph.svg`
- **Performance**: 1000 games
- **View with**: Open SVG in browser
- **Requires**: `cargo install flamegraph`

**Note**: Same permission requirements as perf - use `callgrindprofile` in containers.

#### Allocation Profiling

##### 1. DHAT Profiling (RECOMMENDED for allocations)

```bash
make dhatprofile
```

**Best for**: Rust-native allocation profiling with full symbol information
- **Uses**: `bench` profile (optimized + debuginfo)
- **Works in**: Any environment
- **Output**: `experiment_results/dhat-heap.json`
- **Performance**: 100 rewind iterations to isolate forward gameplay allocations
- **View with**:
  - Interactive: https://nnethercote.github.io/dh_view/dh_view.html (load JSON)
  - Terminal: `python3 scripts/analyze_dhat.py`

**Advantages:**
- Full Rust function names and source locations
- Per-call-site allocation breakdowns with exact file:line references
- Shows allocation hotspots, not just total memory
- Interactive visualization with flame graphs

**Output example:**
```
#1: 90.82 KB (8.4%) in 3,100 blocks (avg 30.0 bytes/block)
  Location: mtg_forge_rs::game::game_loop::GameLoop::get_available_spell_abilities
  (src/game/game_loop.rs:3041:18)
    ↳ GameLoop::priority_round (src/game/game_loop.rs:2150:38)
```

##### 2. Heaptrack Profiling (Alternative)

```bash
make heapprofile
```

**Best for**: System-level allocation tracking
- **Uses**: `--release` build
- **Works in**: Systems with heaptrack installed
- **Output**: `experiment_results/heaptrack.profile.*.gz`
- **Performance**: 100 games
- **Requires**: `apt-get install heaptrack` and `cargo install cargo-heaptrack`
- **Analyze with**: `./scripts/analyze_heapprofile.sh`

**Note**: Less detailed than DHAT for Rust code. Use DHAT unless you need system-level view.

### Performance Tracking Over Time

```bash
# Automatically run benchmarks when git depth advances by 5+ commits
./scripts/periodically_run_benchmarks.sh

# View performance trends
./scripts/plot_performance.py
```

**Output**: `experiment_results/<CPU>/perf_history.csv` with historical data for all key metrics.

### Current Profiling Results

**Top CPU Hotspots** (from Callgrind profiling of 250 games, 10.9B instructions):

1. **ManaEngine::update** (30.2%) - Mana source calculation and tracking
2. **cast_spell_8_step** (23.6%) - Full spell casting pipeline
3. **tap_for_mana_for_cost** (12.6%) - Mana payment execution
4. **GreedyManaResolver::check_payment** (12.3%) - Payment validation
5. **core::fmt::write** (11.8%) - String formatting for logging
6. **alloc::fmt::format_inner** (11.8%) - String allocation for logging

**Key insight**: Logging/formatting accounts for ~23% of CPU time even in "silent" mode due to string construction.

**Top Allocation Hotspots** (from DHAT profiling of 100 iterations, 1.05 MB total):

1. **get_available_spell_abilities** (8.3%) - Vec allocations per priority round
2. **UndoLog::log** (4.4%) - Undo log growth during gameplay
3. **ManaEngine::update** (1.7%) - Mana source tracking
4. **get_castable_spells** (1.7%) - Spell filtering operations
5. **resolve_spell operations** (3.2% combined) - Stack resolution

**Key insight**: Spell ability enumeration allocates heavily on every priority round. Consider caching when board state is unchanged.

## Common Anti-Patterns to Avoid

### 1. Returning Fresh Collections

```rust
// ❌ BAD: Allocates new Vec every call
pub fn get_creatures(&self) -> Vec<CardId> {
    self.battlefield.iter()
        .filter(|c| c.is_creature())
        .map(|c| c.id)
        .collect()
}

// ✅ GOOD: Returns iterator over existing data
pub fn get_creatures(&self) -> impl Iterator<Item = CardId> + '_ {
    self.battlefield.iter()
        .filter(|c| c.is_creature())
        .map(|c| c.id)
}
```

### 2. Cloning to Satisfy the Borrow Checker

```rust
// ❌ BAD: Clone to avoid borrow checker
let cards = self.hand.clone();
self.do_something_that_mutates();
for card in cards { /* ... */ }

// ✅ GOOD: Collect IDs first (smaller), or restructure
let card_ids: Vec<_> = self.hand.iter().map(|c| c.id).collect();
self.do_something_that_mutates();
for id in card_ids {
    let card = self.get_card(id);
    /* ... */
}
```

### 3. Unnecessary String Allocations

```rust
// ❌ BAD: Creates temporary String
fn log_card(&self, card: &Card) {
    println!("Card: {}", card.name.clone());
}

// ✅ GOOD: Borrow string directly
fn log_card(&self, card: &Card) {
    println!("Card: {}", card.name);
}
```

### 4. Collecting Then Chaining

```rust
// ❌ BAD: Collect then iterate again
let creatures: Vec<_> = cards.iter().filter(|c| c.is_creature()).collect();
let untapped: Vec<_> = creatures.iter().filter(|c| !c.is_tapped()).collect();

// ✅ GOOD: Chain without intermediate collection
let untapped_creatures = cards.iter()
    .filter(|c| c.is_creature())
    .filter(|c| !c.is_tapped());
```

## Status and Backlog

### Active Optimization Work

For current allocation hotspots, profiling results, and optimization tasks, see the tracking issue:

**Issue mtg-2**: Optimization and performance tracking

Run `bd show mtg-2` to view current status, or use `make heapprofile` to generate fresh profiling data.

The tracking issue contains up-to-date heap profiling results with specific file:line references and prioritized optimization opportunities.

### Optimization Wins

Track completed optimizations and their measured impact here:

#### 1. Conditional Compilation of Logging (mtg-6) - commit#165

**Problem**: String formatting in logging was the #1 allocation hotspot (70%+ of allocations):
- 77,378 calls in Combat.clear() logging
- 45,274 calls in draw card logging
- 43,437 calls in discard logging
- Every `format!()` macro allocates even when verbosity level is `Silent`

**Solution**: Implemented compile-time feature flag `verbose-logging`:
- Created `log_if_verbose!()` macro that conditionally compiles logging code
- When feature is disabled, logging is completely eliminated at compile time (zero cost)
- Enabled by default for backward compatibility

**Results**:
- With feature enabled (default): Behavior unchanged, ~607-633 games/sec
- With feature disabled (`--no-default-features`): Eliminates ALL logging allocations
- Sets pattern for future zero-cost conditional features

**Usage**:
```bash
# Performance benchmarks without logging:
cargo bench --no-default-features
```

**Files modified**:
- `Cargo.toml`: Added `verbose-logging` feature (default)
- `src/game/game_loop.rs`: Added macro and replaced 5 high-frequency logging calls

## Future Directions

### Rewind/Undo System

The PROJECT_VISION.md describes plans for an undo log system to enable efficient game tree search. This will be critical for AI development and should be designed with zero-copy principles:

- Use unboxed enum for `GameAction` variants
- Store in contiguous `Vec` or arena
- Compile-time flag to disable undo logging for pure replay benchmarks
- Minimize action granularity (what's the minimum state change needed?)

### Compile-Time Flags for Profiling

Consider feature flags for different optimization profiles:
- `zero-copy-strict`: Enforce at compile time (return impl Iterator, deny clone in certain modules)
- `undo-logging`: Enable/disable undo log overhead
- `debug-allocations`: Track allocation sites for profiling

## References

- [The Rust Performance Book](https://nnethercote.github.io/perf-book/)
- [Rust Performance Pitfalls](https://llogiq.github.io/2017/06/01/perf-pitfalls.html)
- [Arenas in Rust](https://manishearth.github.io/blog/2021/03/15/arenas-in-rust/)
- [Zero-Copy in Rust (CoinsBench)](https://coinsbench.com/zero-copy-in-rust-challenges-and-solutions-c0d38a6468e9)


---

**Note**: This is a living document. Update it as we discover new patterns, complete optimizations, or identify new bottlenecks through profiling.
