---
title: Optimization and performance tracking
status: open
priority: 1
issue_type: epic
labels:
- tracking
created_at: 2025-10-26T21:06:34+00:00
updated_at: 2025-11-29T22:34:58.944478693+00:00
---

# Description

Track performance optimization work for MTG Forge Rust.

## Linux Perf Profiling Infrastructure (2025-11-29_#966)

✅ **Perf profiling working in container** (a1365f5, a05ecc6)
- Removed sudo requirement (CAP_PERFMON + CAP_SYS_ADMIN capabilities)
- Fixed rewind_bench CLI arguments (--sequential → -m sequential)
- Added explicit output file (-o perf.data) to prevent piping issues
- Updated docs/PERF_PROFILING_PODMAN.md with status

✅ **Wall-clock CPU hotspot analysis complete** (2025-11-29_#966)
- 5000 games benchmark: 2,838 games/sec (1.8s CPU, 5.4s total)
- 3,160 samples, 8.09 billion cycles analyzed
- Complements Callgrind (instruction count) with wall-clock view
- See ai_docs/perf_cpu_profiling_2025-11-29.md for detailed analysis

**Perf vs Callgrind comparison:**

| Tool | Measurement | Top Hotspot | Use Case |
|------|-------------|-------------|----------|
| **Perf** | Wall-clock time (sampling) | priority_round (17.75%) | Real-world performance |
| **Callgrind** | Instruction count (simulation) | ManaEngine::update (30.5%) | Micro-optimization |

**Key findings from perf profiling:**
- priority_round (17.75%) dominated by HashMap lookups + action generation
- ManaEngine::update (15%) confirms Callgrind findings
- String operations (8.89% combined) - mostly memory-bound
- EntityStore HashMap overhead appears across multiple call sites (~3%)

**Optimization roadmap updated in mtg-166:**
- Quick wins (10-13% CPU): Cache flags, reuse buffers, complete bitfield migration
- Medium impact (10-15% CPU): Cache actions, EntityStore Vec indices
- High risk/reward (5-7% CPU): Incremental ManaTracker caching

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
- Update mtg-162 with allocator comparison results

---

**Current performance as of 2025-11-07_#823(b33ddea0) - Vec<ManaColor> ELIMINATED:**

*Old School deck (03_robots_jesseisbak.dck mirror, 10 games, stats_alloc):*
- **Rewind + Play Again**: **2,120.62 games/sec** (baseline: 2,104.01 games/sec)
- **Performance gain**: +0.79% throughput improvement
- **Total allocations**: 665,437 bytes (was 963,317, **-30.9% allocation reduction**)
- **Avg bytes/game**: 66,543.70 (was 96,331.70, -29,788 bytes/game)
- **Bytes/action**: 121.99 (was 176.59, -54.60 bytes/action)

*Old School deck (03_robots_jesseisbak.dck mirror, 1000 games - previous):*
- **Rewind + Play Again**: **109.29 games/sec** (+250% vs baseline 31.20 games/sec!)
- **Duration**: 9.150s (was 32.052s, -71.4% time)
- **Actions throughput**: 63,943 actions/sec (was 18,260, +250%)
- **Total turns**: 22,153 turns, 585,063 actions

**Latest allocation profiling (2025-11-07_#823, 10 games rewind+replay, stats_alloc):**

Total allocations: **665 KB** (was 963 KB before ManaColors, was 86.4 MB with DHAT profiling)
- **30.9% allocation reduction from Vec<ManaColor> elimination!**
- Expected similar improvement at scale (1000 games)

Top remaining hotspots (post-Vec<ManaColor> elimination):
1. ~~mana_engine.rs:550 - Vec growth in dual land parsing~~ **✅ ELIMINATED (ManaColors bitfield)**
2. random_controller.rs:103 - Random choice generation: ~5% of remaining
3. mana_payment.rs:580 - Mana source selection: ~3% of remaining
4. game_loop.rs:1413 - Game state updates: ~2.5% of remaining

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
- ✅ mtg-156: RNG bincode serialization (96 bytes/turn saved, ~8% of advance_step allocations)
- ✅ mtg-160: RNG SmallVec inline storage (heap allocation eliminated, +52% Rewind mode performance!)
- ✅ mtg-161: Parallel benchmark implementation (exposed allocator contention bottleneck)
- ✅ mtg-165: String allocation cache (CardCache + AbilityCache) - **94.2% allocation reduction (1.48 GB → 86.4 MB), 3.5x speedup (31.20 → 109.29 games/sec)**
- ✅ **Vec<ManaColor> bitfield optimization (b33ddea0)** - **30.9% allocation reduction (963 KB → 665 KB per 10 games), ManaProductionKind now Copy**
- ✅ **ManaEngine::update Vec pre-allocation (03afd440)** - Eliminated Vec reallocation hotspot from DHAT top 20 (was #17 at 18.75 KB)
- ✅ **Bump allocator infrastructure (881f9a06, 7af8fc68)** - Added nightly allocator_api feature + Bump in GameState for future arena allocation
- ✅ **get_available_spell_abilities zero-allocation refactor (cc155429, 7af8fc68)** - Eliminated 3 intermediate Vecs:
  - lands_in_hand_iter: returns iterator instead of Vec
  - push_castable_spells: writes directly to abilities_buffer
  - push_activatable_abilities: writes directly to abilities_buffer
- ✅ **EntityStore::try_get() optimization (7f53776c, 2025-11-28_#957)** - **10-13% CPU speedup**
  - Added try_get() returning Option<&T> instead of Result<&T, MtgError>
  - Eliminates Result/MtgError drop overhead in hot paths
  - Callgrind showed drop_in_place<Result<&Card, MtgError>> was 14% of CPU
  - Updated: ManaEngine::update, GameStateView, game_loop/actions.rs
- ✅ **abilities_buffer reuse optimization (576e6a95, 2025-11-28_#959)** - **8% total allocation reduction**
  - Changed get_available_spell_abilities() to return &[SpellAbility] instead of Vec
  - Buffer is cleared and reused (retains capacity) instead of moved out via mem::take
  - Eliminated push_castable_spells hotspot (~5.3% of allocations)
  - Bytes/action: 48.01 → 40.29 (-16.1%)
- ✅ **GameLoop/ManaEngine pre-allocation (34caa0c, 2025-11-29_#965)** - **11.1% speedup**
  - Pre-allocate abilities_buffer (capacity 16) in GameLoop::new()
  - Pre-allocate ManaEngine vectors (capacity 8) in ManaEngine::default()
  - Reuse self.mana_engine in priority.rs instead of creating new per ability
  - Reduced allocation blocks: 24,425 → 24,027 (-398 blocks, -1.6%)

**Parallel optimization infrastructure:**
- ✅ Pinned thread pool for precise parallel timing
- ✅ Jemalloc allocator support
- ✅ Parallel speedup analysis script
- 🚧 mtg-162: Epic tracking parallel MCTS optimization (allocator contention fixes)

**Infrastructure:**
- ✅ Workspace refactoring (split into mtg-engine + mtg-benchmarks packages)
- ✅ CI fixes for workspace structure
- ✅ Parallel benchmark refactoring (win rate tracking, cleaner code)

**Low priority (remaining allocations):**
- ~~GameLoop::get_available_spell_abilities helper allocations - 51KB (4.6%)~~ **✅ ELIMINATED**
  - ~~get_lands_in_hand, get_castable_spells return Vecs~~
  - Refactored to push directly to abilities_buffer (zero intermediate allocations)
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
1. ~~Pre-size Vec at mana_engine.rs:550~~ **✅ DONE (b33ddea0)** - Replaced Vec with ManaColors bitfield: 30.9% allocation reduction
2. ~~ManaEngine::update Vec reallocations~~ **✅ DONE (03afd440)** - Pre-allocated Vec capacity, eliminated from DHAT top 20
3. Investigate random_controller.rs:103 - Random choice generation (~5% of remaining allocations)
4. Investigate mana_payment.rs:580 - Mana source selection (~3% of remaining)
5. Investigate game_loop.rs:1413 - Game state updates (~2.5% of remaining)

See OPTIMIZATION.md for detailed patterns and profiling methodology.
See experiment_results/dhat_allocation_analysis_2025-11-07_#822.md for complete analysis.

---
**Updated 2025-11-29_#965(34caa0c)** - GameLoop/ManaEngine pre-allocation: 11.1% speedup, 398 fewer allocation blocks
