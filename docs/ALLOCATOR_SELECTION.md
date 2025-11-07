# Allocator Selection: Compile-Time vs Runtime

## Current Implementation (Compile-Time Selection)

The mtg-benchmarks package currently supports **compile-time allocator selection** via Cargo feature flags:

```bash
# System allocator with allocation tracking (DEFAULT)
cargo run --release -p mtg-benchmarks --bin rewind_bench

# mimalloc - high performance allocator
cargo run --release -p mtg-benchmarks --bin rewind_bench --no-default-features --features bench-mimalloc

# jemalloc - high performance allocator with optional stats
cargo run --release -p mtg-benchmarks --bin rewind_bench --no-default-features --features bench-jemalloc

# DHAT heap profiling
cargo run --release -p mtg-benchmarks --bin rewind_bench --no-default-features --features dhat-heap -- --dhat
```

### Available Allocators

| Allocator | Feature Flag | Use Case | Tracking | Performance |
|-----------|-------------|----------|----------|-------------|
| **stats_alloc** | `bench-stats-alloc` (default) | Development, debugging | ✅ Full allocation tracking | Baseline (system malloc + tracking overhead) |
| **mimalloc** | `bench-mimalloc` | Production, benchmarking | ❌ No tracking | High (5-10% faster than system) |
| **jemalloc** | `bench-jemalloc` | Production, benchmarking | ⚠️ Optional via API | High (similar to mimalloc) |
| **DHAT** | `dhat-heap` | Heap profiling | ✅ Comprehensive profiling | Low (profiling overhead) |

### Performance Comparison (10 games, sequential)

From quick benchmark (2025-11-07):

- **stats_alloc**: 2,059 games/sec (baseline)
- **mimalloc**: 2,165 games/sec (+5.1% faster)
- **jemalloc**: 2,177 games/sec (+5.7% faster)

For longer runs with the string cache optimization (1000 games):
- **stats_alloc**: ~109 games/sec
- Expected **mimalloc/jemalloc**: ~115 games/sec (5-10% improvement)

---

## Runtime Allocator Selection

### What It Would Take to Implement

Runtime allocator selection would require significant architectural changes:

#### 1. **Global Allocator Limitation** (BLOCKER)

Rust's `#[global_allocator]` attribute must be set **at compile time**. There is **no safe way** to change the global allocator at runtime.

```rust
// This MUST be compile-time constant
#[global_allocator]
static GLOBAL: SomeAllocator = SomeAllocator;
```

**Why**: The global allocator is deeply integrated into the Rust runtime and standard library. Changing it at runtime would require:
- Invalidating all existing allocations
- Migrating memory between allocators (impossible without double-free or use-after-free)
- Synchronizing allocator swap across all threads

#### 2. **Possible Workarounds** (All Have Severe Tradeoffs)

##### Option A: Manual Allocation Layer (COMPLEX)

Create a custom allocator that dispatches to different backends:

```rust
struct RuntimeSelectableAllocator {
    backend: AtomicPtr<dyn Allocator>, // NOT SAFE - Allocator is not object-safe!
}
```

**Problems**:
- `std::alloc::Allocator` trait is **not object-safe** (cannot use `dyn Allocator`)
- Would need to implement trait manually for each backend
- Cannot change backend after any allocations exist
- Massive performance overhead from indirection

**Verdict**: ❌ Not feasible due to object-safety and safety concerns

##### Option B: Compile Multiple Binaries (PRACTICAL)

Build separate binaries for each allocator:

```bash
# Build all variants
cargo build --release --bin rewind_bench --features bench-stats-alloc
cargo build --release --bin rewind_bench --no-default-features --features bench-mimalloc
cargo build --release --bin rewind_bench --no-default-features --features bench-jemalloc

# Run with launcher script
./run_with_allocator.sh mimalloc -- -n 1000 --mode par
```

**Pros**:
- ✅ Zero runtime overhead
- ✅ Simple to implement (just a shell script)
- ✅ Maintains compile-time optimizations
- ✅ Works with all allocators including DHAT

**Cons**:
- ❌ Larger binary storage (3-4 variants)
- ❌ More complex build process
- ❌ User must select allocator before running

**Verdict**: ✅ **RECOMMENDED** - This is what the parallel_speedup_analysis.sh script does

##### Option C: Dynamic Library Loading (ADVANCED)

Load allocator as a shared library:

```bash
LD_PRELOAD=/path/to/libjemalloc.so ./rewind_bench
```

**Pros**:
- ✅ Single binary
- ✅ Runtime selection via environment variable
- ✅ Works with system tools (LD_PRELOAD)

**Cons**:
- ❌ Platform-specific (Linux/macOS only)
- ❌ Doesn't work with static linking
- ❌ Doesn't work with DHAT (requires compile-time setup)
- ❌ Cannot track allocations (stats_alloc requires compile-time setup)
- ❌ May miss some allocations made before .so is loaded

**Verdict**: ⚠️ Limited use case - only for switching between system/mimalloc/jemalloc

#### 3. **Hybrid Approach** (CURRENT SOLUTION)

Use **compile-time selection** with **build-time automation**:

```bash
# Helper script: run_with_allocator.sh
#!/bin/bash
ALLOCATOR=$1
shift

case $ALLOCATOR in
    stats)   FEATURES="bench-stats-alloc" ;;
    mimalloc) FEATURES="bench-mimalloc" ;;
    jemalloc) FEATURES="bench-jemalloc" ;;
    dhat)    FEATURES="dhat-heap" ;;
    *)       echo "Unknown allocator: $ALLOCATOR"; exit 1 ;;
esac

cargo run --release -p mtg-benchmarks --bin rewind_bench \
    --no-default-features --features $FEATURES -- "$@"
```

**Usage**:
```bash
./run_with_allocator.sh mimalloc -- -n 1000 --mode par
./run_with_allocator.sh jemalloc -- --threads 16
./run_with_allocator.sh dhat -- --dhat
```

---

## Tradeoffs Summary

### Compile-Time Selection (CURRENT)

**Pros**:
- ✅ **Zero runtime overhead** - allocator calls are direct, not through vtable
- ✅ **Compile-time optimizations** - LLVM can inline allocator calls
- ✅ **Type safety** - impossible to mix allocations from different allocators
- ✅ **Works with all allocators** - including DHAT which requires special setup
- ✅ **Simple implementation** - just feature flags in Cargo.toml

**Cons**:
- ❌ **Requires recompilation** - changing allocator needs rebuild (30-60 seconds)
- ❌ **Binary size** - need separate binaries for each allocator (if pre-built)
- ❌ **User friction** - need to remember feature flags

### Runtime Selection (HYPOTHETICAL)

**Pros**:
- ✅ **No recompilation** - change allocator with a flag
- ✅ **Single binary** - simpler distribution
- ✅ **User-friendly** - just `--allocator=mimalloc`

**Cons**:
- ❌ **Impossible in safe Rust** - global allocator must be compile-time constant
- ❌ **Performance overhead** - indirection through function pointer (5-15% slowdown)
- ❌ **Doesn't work with DHAT** - profiling allocator requires compile-time setup
- ❌ **Doesn't work with stats_alloc** - tracking requires compile-time wrapping
- ❌ **Complex implementation** - requires custom allocator trait + dispatch layer
- ❌ **Unsafe code** - would require extensive unsafe blocks for memory management

---

## Recommendations

### For Development/Debugging
Use **stats_alloc** (default) for allocation tracking:
```bash
cargo run --release -p mtg-benchmarks --bin rewind_bench
```

### For Performance Benchmarking
Use **mimalloc** or **jemalloc**:
```bash
cargo run --release -p mtg-benchmarks --bin rewind_bench \
    --no-default-features --features bench-mimalloc -- -n 10000
```

### For Heap Profiling
Use **DHAT**:
```bash
cargo run --release -p mtg-benchmarks --bin rewind_bench \
    --no-default-features --features dhat-heap -- --dhat -n 1000
```

### For Automated Testing (Parallel Allocator Comparison)
Use the existing **parallel_speedup_analysis.sh** script which:
- Builds with each allocator
- Runs benchmarks across thread counts (1, 2, 4, 8, 16, ...)
- Collects CSV data for plotting
- Compares allocator performance

```bash
./scripts/parallel_speedup_analysis.sh
# Generates: experiment_results/parallel_speedup_TIMESTAMP.csv
```

---

## Future Considerations

### Custom Arena Allocator (Per-Thread Bump Allocators)

For parallel MCTS, we may want **per-thread arena allocators**:

```rust
struct ThreadLocalArena {
    buffer: Vec<u8>,
    offset: usize,
}

impl ThreadLocalArena {
    fn alloc<T>(&mut self, value: T) -> &mut T {
        // Bump allocate from thread-local buffer
        // Reset on game completion
    }
}
```

**Benefits**:
- ✅ **No contention** - each thread has its own allocator
- ✅ **Fast allocation** - just bump a pointer
- ✅ **Fast deallocation** - reset pointer to buffer start
- ✅ **Cache-friendly** - allocations are contiguous

**Tradeoffs**:
- ❌ **Requires extensive API changes** - must use arena-allocated types
- ❌ **Lifetime complexity** - arena-allocated objects must outlive arena
- ❌ **Not a drop-in replacement** - requires refactoring game logic

This would be implemented **in addition to** the global allocator, not as a replacement.

---

## Conclusion

**Runtime allocator selection is not practical or necessary** for this project:

1. **Compile-time selection is fast enough** - rebuilding takes 30-60 seconds
2. **Performance overhead is unacceptable** - 5-15% slowdown defeats the purpose
3. **Cannot support all allocators** - DHAT and stats_alloc require compile-time setup
4. **Build scripts suffice** - `parallel_speedup_analysis.sh` automates testing all allocators
5. **Safety concerns** - runtime selection would require extensive unsafe code

**Current solution** (compile-time + build scripts) provides the best tradeoff:
- Zero runtime overhead
- Supports all allocators (stats_alloc, mimalloc, jemalloc, DHAT)
- Simple to use with helper scripts
- Maintains type safety and compile-time optimizations

**Recommendation**: Keep the current compile-time selection approach. If user friction is a concern, provide shell wrapper scripts for common use cases.
