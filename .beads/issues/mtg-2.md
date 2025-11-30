---
title: Optimization and performance tracking
status: open
priority: 1
issue_type: epic
labels:
- tracking
created_at: 2025-10-26T21:06:34+00:00
updated_at: 2025-11-30T17:59:14.982278592+00:00
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

## Latest Optimization (2025-11-30_#1011)

✅ **Skip target validation for non-targeting abilities (49dcde6)** - **~2.3% speedup**
- Check requires_target flag BEFORE calling get_valid_targets_for_ability()
- Non-targeting abilities (firebreathing, regeneration) now skip expensive check
- Benchmark: robots_mirror/rewind_play_again -2.3% execution time (p < 0.05)
- Reference: push_activatable_abilities was 3.16% of CPU (perf profiling)

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
- ✅ **Skip target validation for non-targeting abilities (49dcde6, 2025-11-30_#1011)** - **~2.3% speedup**

**Next high-priority optimizations:**
1. Cache available actions (OPT-NEW-1) - invalidate on state change (5% potential)
2. EntityStore Vec indices (OPT-NEW-2) - replace HashMap with Vec (3% potential)
3. Pre-compute castability flags (OPT-NEW-3) - additional caching (2% potential)

See OPTIMIZATION.md for detailed patterns and profiling methodology.
See experiment_results/reports/perf_cpu_profiling_2025-11-29.md for CPU hotspot analysis.

---
**Updated 2025-11-30_#1011(49dcde6)** - Skip target validation: ~2.3% speedup
