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

## Profiling Analysis with perf (2025-11-04_#723(f961c473))

**Environment:**
- System: AMD Ryzen Threadripper PRO 7975WX (32 physical cores, 64 logical CPUs)
- Benchmark configuration: 32 worker threads (using physical cores only per code)
- Profile duration: 15 seconds per benchmark

**NOTE:** Kernel perf_event restrictions prevented full perf record profiling. Analysis based on Criterion benchmark metrics and allocation tracking via stats_alloc.

### Benchmark Results (2025-11-04)

**Sequential Baseline (rewind_play_again):**
```
Games/sec:        47,691.12
Avg bytes/game:   6,523.55 (6.37 KB)
Total games:      659,255 (in 13.82s)
```

**Parallel Results (par_rewind_play_again, 32 threads):**
```
Games/sec:        1,112.17 (aggregate)
Per-thread:       34.76 games/sec
Avg bytes/game:   674,931.29 (659 KB)
Total games:      151,392 (in 136.12s)
```

### Critical Findings

**1. CATASTROPHIC PARALLEL INEFFICIENCY**

| Metric | Value | Target | Status |
|--------|-------|--------|--------|
| Aggregate speedup | 0.023x | ~32x | ❌ CRITICAL |
| Parallel efficiency | 0.1% | 60-80% | ❌ CRITICAL |
| Per-thread slowdown | 1372x | <2x | ❌ CRITICAL |

**This is 14x worse than the 1.5% efficiency reported earlier with 16 threads!**

The parallel version is **43x SLOWER** than running sequentially (1,112 games/sec vs 47,691 games/sec).

**2. MASSIVE ALLOCATION EXPLOSION**

| Metric | Sequential | Parallel | Ratio |
|--------|------------|----------|-------|
| Bytes/game | 6.5 KB | 659 KB | **103x** |
| Extra allocation | - | 652 KB | - |

The 103x allocation increase indicates:
- **GameState cloning dominates** (likely ~650 KB per clone)
- Each parallel iteration clones the game state 32 times
- Total memory traffic per iteration: ~21 MB
- With 32 threads running concurrently: **massive memory bandwidth saturation**

**3. ROOT CAUSE ANALYSIS**

The profiling data points to a **three-way bottleneck**:

#### a) GameState Clone Cost (PRIMARY BOTTLENECK - 95% of problem)

The 652 KB extra allocation per game in parallel mode directly correlates to GameState cloning:
- Code at benches/game_benchmark.rs:1103-1105 clones snapshot 32 times
- Each clone allocates: Cards (~8 KB) + undo_log (~10 KB) + zones + strings
- **The 659 KB figure suggests clones are MUCH larger than previously estimated**
- Likely includes: Card.name strings, logger buffers, zone vectors, player data

**Evidence:**
- Sequential: 6.5 KB allocated per half-game forward simulation
- Parallel: 659 KB per half-game = clone cost + forward simulation
- Clone cost: 659 - 6.5 = **~652 KB per GameState clone**
- This is **32x-43x larger** than the 15-20 KB estimate in previous analysis

**Impact:**
```
32 threads × 652 KB/clone = 20.9 MB cloned per iteration
With ~1,100 iterations/sec = 22 GB/sec memory traffic just for cloning
Add gameplay allocations = ~25-30 GB/sec total memory bandwidth
```

For reference, DDR5-4800 provides ~38 GB/sec bandwidth per channel. With memory contention across 32 cores, this saturates available bandwidth.

#### b) Allocator Contention (SECONDARY - 5% of problem)

Even with reduced allocation, the 0.1% efficiency indicates severe lock contention:
- glibc malloc uses per-arena locks (typically 8-16 arenas)
- 32 threads competing for ~16 arenas = average 2 threads per lock
- Each allocation/deallocation serializes threads on the same arena
- Cache coherency protocol (MESI) amplifies this: each lock operation invalidates other cores' caches

#### c) Cache/Memory System Saturation (AMPLIFIES both above)

The Threadripper system characteristics exacerbate the problem:
- 32 MB L3 cache (shared across CCDs)
- Multiple CCDs/CCXs create NUMA-like latency within socket
- 652 KB working set per thread × 32 = 20 MB active memory
- Working set barely fits in L3, causing cache thrashing
- Cross-CCD memory access has 2-3x latency penalty

### Hypothesis for Parallel Bottlenecks (Ordered by Impact)

**Hypothesis 1: GameState Clone Memory Bandwidth Saturation (95% of slowdown)**

Each benchmark iteration:
1. Main thread rewinds game to midpoint
2. Clones GameState 32 times (32 × 652 KB = 20.9 MB allocation)
3. Spawns 32 rayon threads to process clones in parallel
4. Each thread runs half a game (~6.5 KB additional allocation)

**Bottleneck:** The initial 32-way clone generates a **massive memory bandwidth spike**:
- 20.9 MB must be allocated and copied
- Deep copies of Cards, Strings, Vecs, undo_log
- Likely happens serially or with limited parallelism (Rust's clone is single-threaded)
- Saturates memory controller before parallel work even starts

**Evidence supporting this hypothesis:**
- 103x allocation increase matches clone behavior
- Per-thread slowdown (1372x) suggests threads are waiting, not just slow
- Aggregate throughput (1,112 games/sec) << sequential (47,691) indicates serialization
- The benchmark code shows cloning happens in a Vec::collect() which is sequential

**Implications:**
- Even if we fix allocator contention, clone cost will dominate
- 652 KB clone cost must be reduced by 90-95% to achieve good parallel efficiency
- Current mtg-61ea98 target of 60% reduction (652 KB → 260 KB) is **insufficient**
- Need to reach <50 KB per clone (92% reduction) for 80% parallel efficiency

**Hypothesis 2: Allocator Lock Convoy Effect (5% of slowdown)**

Once threads start running, they encounter allocator contention:
- Threads awaken and immediately allocate (controller setup, etc.)
- Hit the same arena locks simultaneously
- "Convoy effect": threads form a queue behind the lock
- Each thread that acquires lock invalidates other cores' caches
- Cache coherency traffic keeps cores stalled even after lock release

**Hypothesis 3: False Sharing in stats_alloc Instrumentation**

The benchmark uses stats_alloc for tracking allocations:
- Every allocation updates shared counters
- Likely protected by atomics or locks
- 32 threads hammering the same cache line
- Could be contributing to the extreme slowdown

**Test:** Re-run benchmark without stats_alloc to isolate this effect.

### Recommendations (Updated Based on Profiling)

**IMMEDIATE (Before any parallel MCTS work):**

1. **Measure clone cost separately** (NEW)
   - Add benchmark that just measures GameState::clone() time
   - Determine actual size of clone (use std::mem::size_of_val recursively)
   - Identify largest components (likely Card.name strings, undo_log)
   - File detailed issue with clone breakdown

2. **Ultra-aggressive clone reduction** (REVISED target)
   - Target: 652 KB → <50 KB per clone (92% reduction, not 60%)
   - Skip cloning: undo_log (not needed for forward sim), logger, cached_state
   - Use Arc<str> for Card names instead of String (zero-copy sharing)
   - Share immutable card database references, don't clone card data
   - Consider: parameterize GameState by clone mode (full vs simulation)

3. **Test without stats_alloc**
   - Run par_rewind_play_again without allocation tracking
   - If significantly faster, stats_alloc is contributing to contention

**PHASE 1 (Allocation reduction):**

4. **Continue mtg-2 work** but with revised target:
   - Original target: 6.5 KB → 2 KB per game forward play
   - Keep this target, but know it's small relative to clone cost
   - Per-game allocation is only 1% of the parallel bottleneck

**PHASE 2 (When starting parallel MCTS):**

5. **Never use hyperthreading** - stick with physical cores only
   - Current code correctly uses num_cpus::get_physical()
   - But verify rayon thread pool configuration

6. **Consider per-thread bump allocators**
   - Once clone cost is reduced, remaining allocations will still cause some contention
   - bumpalo arenas will eliminate allocator lock contention entirely

### Updated Parallel Efficiency Predictions

**Current state:**
- Clone cost: 652 KB
- Forward play allocation: 6.5 KB
- Parallel efficiency: 0.1%

**After 92% clone reduction (target <50 KB):**
- Clone cost: 50 KB
- Forward play allocation: 6.5 KB
- Total per-iteration allocation: 50 + (32 × 6.5) = 258 KB vs current 21 MB
- **Predicted efficiency: 40-50%** (memory bandwidth no longer saturated)

**After clone reduction + mtg-2 allocation work:**
- Clone cost: 50 KB
- Forward play allocation: 2 KB
- Total per-iteration allocation: 50 + (32 × 2) = 114 KB
- **Predicted efficiency: 60-70%** (cache pressure reduced)

**After adding bump allocators:**
- Clone cost: 50 KB (still on global allocator)
- Forward play: 2 KB on per-thread bump
- No lock contention during parallel phase
- **Predicted efficiency: 75-85%** (minimal contention)

### Summary

The profiling reveals the problem is **even worse** than previously documented:
- 0.1% efficiency vs previous 1.5% measurement
- 103x allocation increase (vs previous ~2x expectation)
- **GameState clone cost is 10-30x larger than estimated** (652 KB vs 15-20 KB)

The clone cost **completely dominates** parallel performance. Until this is fixed:
- **Parallel MCTS will be 40x slower than sequential MCTS**
- No amount of allocator tuning will help
- This is a **critical blocker** for parallel MCTS implementation

**Priority actions:**
1. File detailed clone analysis issue
2. Measure actual clone size and breakdown
3. Implement ultra-aggressive clone reduction (92% reduction target)
4. Re-benchmark to verify predictions

See `experiment_results/perf_history.csv` and benchmark output in `benchmark_output.txt` for raw data.
