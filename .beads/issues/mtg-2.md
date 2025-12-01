---
title: Optimization and performance tracking
status: open
priority: 1
issue_type: epic
labels:
- tracking
created_at: 2025-10-26T21:06:34+00:00
updated_at: 2025-12-01T21:18:54.265564011+00:00
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

---

## Latest Optimization (2025-12-01_#1085(46851f3))

✅ **Skip string formatting when logging disabled** - **26-81% allocation reduction, 34-70% speedup**
- Add is_choice_logging_active() check to GameLogger
- Skip expensive format!() calls in RandomController when logging disabled
- DHAT showed this was #2 allocation hotspot: 355KB (9.6%) in string formatting
- Results:
  - DHAT: 26.2% fewer bytes, 37.9% fewer allocation calls
  - robots_mirror: 81.6% allocation reduction, 38% speedup
  - simple_bolt: 79.1% allocation reduction, 70% speedup
  - Criterion: -34.5% execution time (p < 0.05)

**Previous (2025-12-01_#1080):**
✅ **ManaEngine::read_from_cache zero-allocation refactor** - **45.7% allocation reduction, -21.7% runtime**
- Eliminated temporary Vec allocations in read_from_cache() by pushing directly to self vectors
- Before: 2.0 MB in 39,916 blocks, After: 1.1 MB in 20,860 blocks
- Benchmark: robots_mirror/snapshot_games -21.7% execution time (p < 0.05)
- DHAT profiling showed this was the #1 allocation hotspot (22.8% of total)

**Previous (2025-12-01_#1079):**
✅ **EntityStore HashMap → Vec optimization (1c6f23e)** - **7-22% speedup across benchmarks**
- Replace FxHashMap<EntityId<T>, T> with Vec<Option<T>> for O(1) indexed lookups
- Eliminates hash computation overhead in hot paths
- Key results:
  - robots_mirror/snapshot_games: **-21.6%**
  - robots_mirror/rewind_play_again: **-17.1%**
  - monoblack_thedeck/rewind: **-16.4%**
  - Rewind: **-12.5%**
  - Average improvement: ~10-15% across all benchmarks

**Previous (2025-12-01_#1075):**
✅ **Cache additional type flags in CardCache (c9edb3e)** - **-17.9% snapshot_games**
- Add is_instant, is_sorcery, is_enchantment, is_aura, is_equipment flags
- Eliminates Vec::contains() and eq_ignore_ascii_case() in type checks

**Previous (2025-11-30_#1013):**
✅ **Cache land subtype flags in CardCache** - **~0.5-1.6% speedup**
- Add has_plains/island/swamp/mountain/forest_subtype flags to CardCache
- Eliminates eq_ignore_ascii_case() calls in tap_for_mana_for_cost hot path

---

## Infrastructure Improvements (2025-11-05_#763)

✅ **Pinned thread pool infrastructure** (6834503b)
✅ **Integrated pinned thread pool into benchmark** (7f4266b1)
✅ **Working directory helper and graceful thread pinning** (aa1d4c0b)
✅ **Thread count parameterization** (f80e759d)
✅ **Parallel speedup analysis script complete** (81e4c0b9)
✅ **Jemalloc allocator support** (7259bc22)
✅ **CI fixed for workspace structure** (eaa7d8dc)
✅ **Benchmark refactoring** (96346c0a)

---

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
- ✅ mtg-165: String allocation cache (CardCache + AbilityCache) - **94.2% allocation reduction (1.48 GB → 86.4 MB), 3.5x speedup**
- ✅ Vec<ManaColor> bitfield optimization (b33ddea0) - **30.9% allocation reduction**
- ✅ ManaEngine::update Vec pre-allocation (03afd440) - Eliminated Vec reallocation hotspot
- ✅ Bump allocator infrastructure (881f9a06, 7af8fc68) - Added nightly allocator_api feature
- ✅ get_available_spell_abilities zero-allocation refactor (cc155429, 7af8fc68)
- ✅ EntityStore::try_get() optimization (7f53776c, 2025-11-28_#957) - **10-13% CPU speedup**
- ✅ abilities_buffer reuse optimization (576e6a95, 2025-11-28_#959) - **8% total allocation reduction**
- ✅ GameLoop/ManaEngine pre-allocation (34caa0c, 2025-11-29_#965) - **11.1% speedup**
- ✅ **Skip target validation for non-targeting abilities (49dcde6)** - **~2.3% speedup**
- ✅ **Land subtype caching in CardCache (2025-11-30_#1013)** - **~0.5-1.6% speedup**
- ✅ **Type flag caching (c9edb3e, 2025-12-01_#1075)** - **-17.9% snapshot_games**
- ✅ **EntityStore Vec optimization (1c6f23e, 2025-12-01_#1079)** - **-7-22% across benchmarks**
- ✅ **ManaEngine::read_from_cache zero-allocation (2025-12-01_#1080)** - **45.7% alloc reduction, -21.7% runtime**
- ✅ **Skip string formatting when logging disabled (46851f3, 2025-12-01_#1085)** - **26-81% alloc reduction, 34-70% speedup**

**Next high-priority optimizations:**
1. Cache available actions (OPT-NEW-1) - invalidate on state change (5% potential)

See OPTIMIZATION.md for detailed patterns and profiling methodology.
See experiment_results/reports/perf_cpu_profiling_2025-11-29.md for CPU hotspot analysis.

---
**Updated 2025-12-01_#1085** - Skip string formatting when logging disabled: 26-81% alloc reduction, 34-70% speedup
