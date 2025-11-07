---
title: Optimization and performance tracking
status: open
priority: 1
issue_type: epic
labels:
- tracking
created_at: 2025-10-26T21:06:34+00:00
updated_at: 2025-11-05T16:41:34.685598643+00:00
---

# Description

Track performance optimization work for MTG Forge Rust.

## ⚠️ CRITICAL: Parallel Bottleneck Discovered

**See mtg-a6ca26 for parallel MCTS optimization plan.**

The new parallel benchmark (mtg-a60157) exposed **catastrophic allocator contention**:
- Parallel aggregate: 0.23x speedup (actually SLOWER than sequential!)
- Per-thread: 68.8x slowdown (1.5% of sequential throughput)
- Parallel efficiency: 1.5% (should be >60%)

**Root cause:** System allocator (glibc malloc) global lock serializes all 16 threads.

**Plan:** Two-phase approach in mtg-a6ca26:
1. Maximize zero-copy patterns (target <2KB/game)
2. Quick win: Try mimalloc/jemalloc (expect 10-30x improvement)
3. Per-thread bump allocators (target 80-90% efficiency)

**Impact on MCTS:** Without fixing this, parallel MCTS will be slower than sequential MCTS!

---

## Infrastructure Improvements (2025-11-05_#763)

✅ **Pinned thread pool infrastructure** (6834503b)
- Created custom thread pool with core affinity (core_affinity crate)
- Thread pinning to physical cores for consistent performance
- Spin barriers for precise synchronization (ready/go flags)
- Last-thread-records-time pattern for microsecond-accurate measurements
- Main thread participates as worker 0 (fork N-1 threads)

✅ **Integrated pinned thread pool into benchmark** (7f4266b1)
- New bench_game_pinned_par_rewind_play_again() function
- Custom thread pool replaces Rayon for precise timing
- Win rate tracking across parallel games
- Ready for performance comparison vs Rayon

✅ **Working directory helper and graceful thread pinning** (aa1d4c0b)
- Added ensure_correct_working_directory() function to fix Criterion resource loading
- Graceful degradation when core_affinity returns insufficient cores
- Fixes benchmarks running in containerized environments

✅ **Thread count parameterization** (f80e759d)
- Benchmark now reads BENCH_NUM_THREADS environment variable
- Falls back to num_physical_cores if not set
- Enables testing parallel speedup across different thread counts

✅ **Parallel speedup analysis script complete** (81e4c0b9)
- Script ready to run full benchmark sweeps with thread count variation
- Support for all three allocators (system, mimalloc, jemalloc)
- CSV output for results + matplotlib plotting
- Dry-run mode verified: 3 allocators × 32 cores = 96 benchmark runs

✅ **Jemalloc allocator support** (7259bc22)
- Added tikv-jemallocator as third allocator option
- Compile-time mutual exclusion with other allocators
- Feature flag: bench-jemalloc
- All four allocator modes tested and working

✅ **CI fixed for workspace structure** (eaa7d8dc)
- Updated GitHub Actions to use `--workspace` flag for tests
- Split clippy into separate package commands to avoid feature conflicts
- Updated run_examples.sh to use `-p mtg-forge-rs` flag

✅ **Benchmark refactoring** (96346c0a)
- Abstracted midgame state initialization
- Added win rate tracking to parallel benchmarks
- Cleared undo log at midpoint (only track 50%-100% gameplay)

🚧 **Ready to run: Parallel speedup analysis**
- All infrastructure complete - benchmark accepts BENCH_NUM_THREADS
- Analysis script tested in dry-run mode
- Ready to run full benchmark sweep (very long-running)
- Alternative: Run limited sweep (fewer thread counts) to validate workflow
- Update mtg-a6ca26 with allocator comparison results

---

**Current performance as of 2025-11-07_#822(855b05d5) - MAJOR IMPROVEMENT:**

*Old School deck (03_robots_jesseisbak.dck mirror, 1000 games):*
- **Rewind + Play Again**: **109.29 games/sec** (+250% vs baseline 31.20 games/sec!)
- **Duration**: 9.150s (was 32.052s, -71.4% time)
- **Actions throughput**: 63,943 actions/sec (was 18,260, +250%)
- **Total turns**: 22,153 turns, 585,063 actions

**Latest DHAT heap profiling (2025-11-07_#822, 1000 games rewind+replay):**

Total allocations: **86.4 MB in 5.6M blocks** (was 1.48 GB in 20.4M blocks)
- **94.2% allocation reduction!**
- **72.4% fewer allocation blocks!**

Top remaining hotspots (post-string-cache):
1. mana_engine.rs:550 - Vec growth in dual land parsing: ~27 MB (31%)
2. random_controller.rs:103 - Random choice generation: 4.68 MB (5.4%)
3. mana_payment.rs:580 - Mana source selection: 2.57 MB (3.0%)
4. game_loop.rs:1413 - Game state updates: 2.20 MB (2.5%)

**Previous performance as of 2025-11-04_#713(1961e96):**

*Simple deck (simple_bolt.dck):*
- **Fresh Mode**: 5,520 games/sec, avg 7 turns/game, 232KB/game, 33.1KB/turn
- **Snapshot Mode**: 19,676 games/sec (3.6x faster via clone)
- **Rewind Mode**: 298,854 games/sec (54.1x faster via undo, +52% vs previous!)

*Old School decks (realistic 32-41 turn games):*
- **Mono Black vs The Deck**: 1,479 games/sec, 32 turns/game, 822KB/game, 25.7KB/turn
- **White Weenie Mirror**: 1,068 games/sec, 41 turns/game, 1.22MB/game, 29.7KB/turn
- **Jeskai Aggro vs Troll Disk**: 1,128 games/sec, 39 turns/game, 1.22MB/game, 31.3KB/turn

**Previous DHAT profiling (2025-11-04_#713, 100 iterations rewind+replay):**

Total allocations: 1.10 MB in 27,968 blocks (-2.6% bytes from previous, +6.2% blocks)
Top hotspots:
1. GameLoop::get_available_spell_abilities - ~51KB (4.6%) - helper function allocations
2. Allocator overhead entries (~7-8% each, expected)

**Major wins achieved:**
- ✅ **String allocation cache (CardCache + AbilityCache)**: 1.48 GB → 86.4 MB (-94.2%!)
  - Eliminated repeated to_lowercase() calls (900 MB saved)
  - Pre-computed boolean flags for mana abilities and targeting
  - 3.5x speedup in game throughput (31.20 → 109.29 games/sec)
- ✅ ManaEngine dynamic allocation: 600KB → 0KB (eliminated from top 20!)
- ✅ ManaEngine::update reserve: 70KB → 0KB (eliminated from top 20!)
- ✅ GameLoop abilities buffer: 89KB → 51KB (-43% reduction)
- ✅ RNG serialization: JSON→bincode, 152→56 bytes per turn (63% reduction, 96 bytes/turn saved)
- ✅ RNG SmallVec: Eliminated heap allocation per turn (~40 allocations saved per game)
- ✅ RNG advance_step hotspot: 150KB → eliminated from top hotspots!
- **Total reduction: From 1.48 GB baseline to 86.4 MB (-94.2% total)**

**Completed optimizations:**
- ✅ mtg-6: Logging allocations (conditional compilation added)
- ✅ mtg-10: Vec reallocations in game loop (SmallVec + fixed arrays)
- ✅ mtg-7: CardDatabase.get_card() returns Arc<CardDefinition>
- ✅ mtg-8: GameStateView already uses borrowing, not cloning
- ✅ mtg-9: CardName and PlayerName use Arc<str>
- ✅ mtg-12: Mana pool calculation optimization (already resolved)
- ✅ mtg-11: Zone transfer operations (investigated, already optimal)
- ✅ mtg-120: ManaEngine allocation hotspot (20-39% reduction, 15-16% faster)
- ✅ mtg-current: ManaResolver Box elimination (3-7% faster)
- ✅ mtg-payment-vecs: Mana payment Vec elimination (85% faster, 1.4M allocations eliminated)
- ✅ mtg-mana-engine-dynamic: ManaEngine dynamic allocation elimination (600KB → 70KB, 3-24% faster)
- ✅ mtg-buffer-reuse: GameLoop + ManaEngine buffer optimization (108KB eliminated, -5% total allocations)
- ✅ mtg-437f88: RNG bincode serialization (96 bytes/turn saved, ~8% of advance_step allocations)
- ✅ mtg-02f1df: RNG SmallVec inline storage (heap allocation eliminated, +52% Rewind mode performance!)
- ✅ mtg-a60157: Parallel benchmark implementation (exposed allocator contention bottleneck)
- ✅ mtg-c66412: String allocation cache (CardCache + AbilityCache) - **94.2% allocation reduction (1.48 GB → 86.4 MB), 3.5x speedup (31.20 → 109.29 games/sec)**

**Parallel optimization infrastructure:**
- ✅ Pinned thread pool for precise parallel timing
- ✅ Jemalloc allocator support
- ✅ Parallel speedup analysis script
- 🚧 mtg-a6ca26: Epic tracking parallel MCTS optimization (allocator contention fixes)

**Infrastructure:**
- ✅ Workspace refactoring (split into mtg-engine + mtg-benchmarks packages)
- ✅ CI fixes for workspace structure
- ✅ Parallel benchmark refactoring (win rate tracking, cleaner code)

**Low priority (remaining allocations):**
- GameLoop::get_available_spell_abilities helper allocations - 51KB (4.6%)
  - get_lands_in_hand, get_castable_spells return Vecs
  - Would require more API refactoring for modest gains
- Card loading string clones (acceptable one-time cost)
- UndoLog growth (~43KB, necessary for rewind functionality)
- Allocator overhead (expected, unavoidable)

**Future considerations:**
- mtg-13: Arena allocation for per-turn temporaries
- mtg-14: Object pools for reusable objects
- mtg-15: Compile-time feature flags for profiling modes

**Optimization status: Outstanding!**
We've achieved a **94.2% reduction in total allocations** (1.48 GB → 86.4 MB) and **3.5x speedup** (31.20 → 109.29 games/sec).

The string allocation cache (CardCache + AbilityCache) had a massive impact by eliminating repeated to_lowercase() calls that were causing 900 MB of allocations. This optimization exceeded predictions (50-60%) by achieving 94.2% reduction.

**Next high-priority optimizations:**
1. Pre-size Vec at mana_engine.rs:550 (15-20 MB savings, 5 min fix)
2. Pre-size other hot Vecs in game_loop.rs (5-10 MB savings, 15 min fix)
3. Convert dual land colors to SmallVec (8-12 MB savings, 30 min fix)

See OPTIMIZATION.md for detailed patterns and profiling methodology.
See experiment_results/dhat_allocation_analysis_2025-11-07_#822.md for complete analysis.

---
**Updated 2025-11-07_#822(855b05d5)** - String allocation cache complete: 94.2% allocation reduction, 3.5x speedup, remaining optimizations identified
