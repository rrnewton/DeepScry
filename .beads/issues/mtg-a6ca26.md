---
title: 'Parallel MCTS optimization: Eliminate allocator contention'
status: open
priority: 1
issue_type: task
created_at: 2025-11-04T20:48:12.406849084+00:00
updated_at: 2025-11-04T21:39:48.828812350+00:00
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

### Quick Win: Thread-Local Allocator (Experiment Results)

**⚠️  EXPERIMENT COMPLETE - DO NOT COMMIT** These changes are for measurement only.

Tested mimalloc as drop-in replacement for system allocator.

**Initial Results with 16 threads (hyperthreading enabled, 2025-11-04):**

| Metric | glibc malloc | mimalloc | Improvement |
|--------|--------------|----------|-------------|
| **Sequential** | 44,920 games/sec | 54,871 games/sec | +22% |
| **Parallel (16 threads)** | 10,441 games/sec | 21,967 games/sec | +110% |
| **Per-thread efficiency** | 1.5% of sequential | 2.5% of sequential | +67% |
| **Aggregate speedup** | 0.23x | 0.40x | Still <1.0x |

**Physical Core Results - 8 threads only (no hyperthreading, 2025-11-04):**

| Metric | glibc malloc | mimalloc | Improvement |
|--------|--------------|----------|-------------|
| **Sequential** | 48,585 games/sec | 52,913 games/sec | +8.9% |
| **Parallel (8 threads)** | 17,775 games/sec | 25,100 games/sec | +41% |
| **Per-thread efficiency** | 36.6% of sequential | 47.4% of sequential | +30% |
| **Aggregate speedup** | 0.37x | 0.47x | Still <1.0x |

**Analysis:**

Physical cores vs hyperthreading:
- Using only 8 physical cores gives **24x better per-thread efficiency** (36.6% vs 1.5% with 16 threads)
- Hyperthreading **massively exacerbates** allocator contention (1.5% efficiency vs 36.6%)
- Each hyperthread pair shares L1/L2 cache, multiplying cache coherency traffic

Mimalloc impact:
- With 16 threads: **2.1x improvement** (110% faster) - dramatic but insufficient
- With 8 threads: **1.4x improvement** (41% faster) - modest improvement
- Hyperthreading amplifies mimalloc's benefit (110% vs 41%) because contention is worse

**Conclusion:**

The problem has two components:

1. **Hyperthreading contention** (most severe): Hyperthreads share L1/L2 cache. Allocator operations generate cache coherency traffic that serializes hyperthreads on the same physical core. **Never use hyperthreading for parallel MCTS.**

2. **High allocation frequency** (still critical): Even with physical cores only, 47.4% efficiency is far below the 80-90% target. The **6.5KB/game allocation rate** generates enough cache traffic to degrade parallel performance significantly.

**Next steps:**
- **MCTS implementation: Use only physical cores** (num_cpus::get_physical())
- Phase 1 (zero-copy) is **critical**: Target <1KB per game
- Phase 2 (bump allocators) will be necessary to reach 80-90% efficiency
- Mimalloc provides modest benefit but is not sufficient alone

### Phase 2: Per-Thread Bump Allocators (Target: 80-90% efficiency)

Once allocations are minimized, switch remaining allocations to thread-local arenas:

1. **Per-turn bump allocations in game engine**:
   - Allocations that don't easily eliminate but don't survive the turn
   - Each thread's game engine bump-allocates per-turn state
   - Clear arena at end of turn (not per-game)
   - Examples: temporary ability lists, combat calculations, effect stacks

2. **Per-game bump allocations for simulation threads**:
   - For parallel simulations, parameterize GameState by allocator
   - Each worker thread runs entire game in thread-local allocator
   - Throw away all memory when game completes
   - Zero contention across threads (no shared allocator state)
   - Extremely fast allocation (bump pointer, no locks)

3. **Thread pool with batching**: Persistent threads reduce spawn overhead
   - Maintain pool of N worker threads
   - Queue batches of simulations (e.g., 100 per batch)
   - Each batch reuses same arena (clear after game, not after turn for simulation threads)
   - Threads never block on allocator - each has independent arena

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

**After physical core optimization + mimalloc (CURRENT):**
- Parallel efficiency: 1.5% (16 threads) → 47.4% (8 physical cores)
- Aggregate throughput: 10,441 (16 threads) → 25,100 games/sec (8 physical cores)
- Per-thread: 653 (16 threads) → 3,138 games/sec (8 physical cores)
- **Using physical cores only is MANDATORY** - never use hyperthreading for MCTS

**Phase 2 completion (8 physical cores):**
- Parallel efficiency: 47.4% → 80-90%
- Aggregate throughput: 25,100 → 340,000-380,000 games/sec
- Per-thread: 3,138 → 42,500-47,500 games/sec (near sequential performance)
- **MCTS will scale effectively across all physical cores**

## Additional Contention Analysis (2025-11-04)

Beyond allocation frequency, **GameState cloning** contributes significantly to poor parallel efficiency:

**Clone cost breakdown**:
- Current: 15-20KB per clone (Cards 8KB + Undo log 10KB + Zones 320B + Other 2KB)
- With 8 threads: 120-160KB cloned per benchmark iteration
- Impact: Cache pressure, TLB misses

**Root causes**:
1. Deep copying all Card structs with String fields
2. Cloning entire undo_log (unnecessary for forward simulations)
3. Allocating new Vecs for all zones

**Optimization path**:
- New issue **mtg-61ea98**: Optimize GameState clone for MCTS
- Target: Reduce clone cost by 60% (15-20KB to 5-8KB)
- Method: Selective cloning (skip undo_log, logger for simulations)

**Combined impact prediction**:
- GameState clone optimization: 60% reduction in clone cost
- Plus mtg-2 allocation reduction (<1KB/game): 85% reduction in gameplay allocations
- **Expected parallel efficiency: 70-80%** (vs current 47.4%)

See `ai_docs/parallel_contention_analysis.md` for full analysis.
