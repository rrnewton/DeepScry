---
title: 'Parallel MCTS optimization: Eliminate allocator contention'
status: open
priority: 1
issue_type: task
created_at: 2025-11-04T20:48:12.406849084+00:00
updated_at: 2025-11-04T20:48:48.290352018+00:00
---

# Description

## Problem: Catastrophic Parallel Slowdown

The new `bench_game_par_rewind_play_again` benchmark (mtg-a60157) reveals **severe allocator contention** that will cripple future parallel MCTS implementation:

**Sequential baseline (1 thread):**
- 179,679 turns/sec
- 44,920 games/sec

**Parallel aggregate (16 threads):**
- 41,763 turns/sec (0.23x speedup - SLOWER than sequential!)
- 10,441 games/sec

**Per-thread in parallel mode:**
- 2,610 turns/sec per thread (only 1.5% of sequential!)
- **68.8x slowdown per thread**
- **Parallel efficiency: 1.5%** (should be >60%)

**Root cause:** System allocator (glibc malloc) uses a global lock. With 16 threads constantly allocating/deallocating, they serialize on this lock, generating massive cache coherency traffic.

See `/tmp/parallel_analysis.md` for detailed analysis.

## Critical Insight for MCTS

This benchmark accurately models the parallel MCTS pattern:
1. Snapshot at decision point
2. Fork N worker threads
3. Each explores different futures
4. Aggregate results

**Without fixing this, parallel MCTS will be SLOWER than sequential MCTS!**

## Two-Phase Optimization Strategy

### Phase 1: Maximize Zero-Copy (Target: <2KB per game, <500 bytes per turn)

Continue the work from mtg-2 to eliminate as much allocation as possible:

1. **Destination-passing style**: Reuse long-lived buffers for temporary computations
   - Pass `&mut Vec<T>` to functions that currently return `Vec<T>`
   - Clear and reuse rather than allocate fresh

2. **More SmallVec usage**: Expand inline storage to avoid heap allocation
   - Identify hot paths that allocate small Vecs
   - Replace with `SmallVec<[T; N]>` where N covers common case

3. **Reference passing**: Convert owned return values to borrowed where possible
   - Return `&[T]` from internal buffers instead of `Vec<T>`
   - Extend lifetimes to allow borrowing

4. **Object pools**: Reuse frequently allocated types
   - Pool of Effect objects
   - Pool of ability activation contexts
   - Pool of combat state structures

**Success metric:** Reduce per-game allocation from 6.5KB to <2KB

### Quick Win: Thread-Local Allocator (Target: 10-30x improvement)

Before implementing Phase 2, evaluate drop-in thread-local allocators:

- **mimalloc**: Microsoft's allocator, easiest drop-in replacement
- **jemalloc**: Facebook's allocator, also excellent for parallelism
- **snmalloc**: Research allocator with message-passing design

Just adding this to Cargo.toml should provide immediate dramatic improvement:
```toml
[dependencies]
mimalloc = { version = "0.1", features = ["override"] }
```

Expected: 10-30x reduction in contention, bringing parallel efficiency from 1.5% to 15-45%.

### Phase 2: Per-Thread Bump Allocators (Target: 80-90% efficiency)

Once allocations are minimized, switch remaining allocations to thread-local arenas:

1. **Per-thread bumpalo arena**: Each MCTS worker gets its own arena
   - All simulation state allocated in arena
   - Zero contention (no shared state between threads)
   - Extremely fast allocation (bump pointer)
   - Bulk deallocation (drop entire arena after simulation batch)

2. **Expand existing bumpalo usage**: We already use it in some places
   - Extend to cover all MCTS search allocations
   - Keep arenas alive across multiple simulations (reuse)

3. **Thread pool with batching**: Persistent threads reduce spawn overhead
   - Maintain pool of N worker threads
   - Queue batches of simulations (e.g., 100 per batch)
   - Each batch reuses same arena (clear between batches)

**Success metric:** Achieve 80-90% parallel efficiency (12-14x speedup on 16 cores)

## Implementation Order

1. **Immediate** (continuation of mtg-2):
   - Complete current allocation optimizations
   - Get below 2KB per game if possible
   - Document remaining allocation hotspots

2. **Quick win** (new issue):
   - Evaluate mimalloc/jemalloc as drop-in replacement
   - Benchmark parallel_rewind_play_again with new allocator
   - If successful (>10x improvement), keep it

3. **MCTS design** (new epic when starting MCTS work):
   - Design MCTS with per-thread bumpalo arenas from the start
   - Each worker owns its arena
   - Benchmark to verify near-linear scaling

## Related Issues

- mtg-2: Main optimization tracking (single-threaded focus so far)
- mtg-a60157: Parallel benchmark implementation (completed, exposed this problem)
- Future: Sub-issues for each phase once work begins

## Expected Outcomes

**Phase 1 completion:**
- Per-game allocation: 6.5KB → <2KB
- Reduced allocator pressure even in single-threaded case

**After quick win (mimalloc):**
- Parallel efficiency: 1.5% → 15-45%
- Aggregate throughput: 10,441 → 100,000+ games/sec (16 threads)
- Per-thread: 653 → 6,000+ games/sec

**Phase 2 completion:**
- Parallel efficiency: 80-90%
- Aggregate throughput: ~600,000+ games/sec (16 threads)
- Per-thread: ~40,000 games/sec (near sequential performance)
- **MCTS will scale effectively across all available cores**
