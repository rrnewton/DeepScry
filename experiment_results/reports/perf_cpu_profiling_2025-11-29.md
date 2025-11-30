# Linux Perf CPU Profiling Analysis (2025-11-29_#966)

## Executive Summary

**Measurement:** Wall-clock time sampling with `perf record` (997 Hz, DWARF call graphs)
**Workload:** 5000 games rewind+replay benchmark (robots mirror, sequential mode)
**Duration:** 1.8s CPU time (5.4s total with I/O)
**Samples:** 3,160 samples, 8.09 billion CPU cycles
**Throughput:** 2,838 games/sec

## Profiling vs Callgrind Comparison

| Metric | Linux Perf (this analysis) | Callgrind (mtg-166) |
|--------|---------------------------|---------------------|
| **Measurement** | Wall-clock time (sampling) | Instruction count (simulation) |
| **Overhead** | ~3x slower (5.4s vs 1.8s) | ~50x slower |
| **Top Hotspot** | priority_round (17.75%) | ManaEngine::update (30.5%) |
| **#2 Hotspot** | ManaEngine::update (15.00%) | cast_spell_8_step (23.3%) |
| **Granularity** | Function-level + inlined | Instruction-level |
| **Use Case** | Real-world performance | Micro-optimization |

**Key Insight:** Perf shows priority_round as #1 (17.75%) while Callgrind showed ManaEngine as #1 (30.5%). This is because:
- **Perf samples wall-clock time** - includes cache misses, branch mispredicts, memory stalls
- **Callgrind counts instructions** - pure CPU work without memory hierarchy effects

Both views are valuable and complementary!

## Top CPU Hotspots (Wall-Clock Time)

### Critical Path (>10% each)

#### 1. `priority_round` - 17.75% (game_loop/priority.rs)

**What it does:** Generates available actions during priority, AI chooses one

**Sub-hotspots:**
- 3.16% - `push_activatable_abilities` (ability generation)
- 2.22% - HashMap probing in EntityStore lookups
- 1.84% - General overhead
- 1.28% - EntityId equality comparisons
- 1.25% - Iterator operations (`Iter::next`)
- 0.81% - Hash table tag matching
- 0.62% - `push_castable_spells`

**Root causes:**
- Repeated EntityStore HashMap lookups (2.22% + 0.81% = 3.03%)
- Scanning cards for abilities/castability
- Building action list from scratch every time

**Optimization opportunities:**
1. Cache available actions (invalidate on state change)
2. Use Vec indices instead of HashMap for hot paths
3. Pre-compute castability flags

#### 2. `ManaEngine::update` - 15.00% (game/mana_engine.rs)

**What it does:** Computes available mana sources for mana payment

**Sub-hotspots:**
- 2.87% - General overhead
- 2.40% - Vec::push operations (allocating results)
- 1.52% - Iterator overhead
- 1.24% - Card type checking (is_creature, is_land)
- 1.15% - String comparisons for mana colors
- 1.14% - Vec::push writes
- 1.12% - HashMap probing for card lookups
- 0.76% - CardType equality checks

**Root causes:**
- Rescans battlefield every call (called 30+ times per spell)
- Vec allocations for intermediate results
- String comparisons for color matching
- Card type checks via slice contains

**Optimization opportunities:**
1. **Incremental caching** - Recompute only on battlefield changes
2. **Pre-sized Vecs** - Avoid reallocation
3. **Cached type flags** - is_creature/is_land as bool flags
4. **Color bitfield** - Replace string comparisons with bitwise ops

**ALREADY DONE (per mtg-2):**
- ✅ Vec<ManaColor> → bitfield (30.9% allocation reduction)
- ✅ Vec pre-allocation (eliminated from DHAT top 20)

**Combined: 32.75% of total CPU time** - These two functions dominate

### Hot Functions (2-5%)

| % Time | Function | File | Issue |
|--------|----------|------|-------|
| 4.79% | `bounds_check_payment` | mana_payment.rs | Validates mana payment |
| 3.25% | `str::is_contained_in` | core/str | String pattern matching |
| 2.78% | `tap_for_mana_for_cost` | actions.rs | Mana tapping logic |
| 2.25% | `get_valid_targets_for_ability` | actions/targeting.rs | Target validation |
| 2.15% | `String::write_str` | alloc/string | String allocation |
| 2.09% | `GameState::undo` | game/state.rs | Rewind mechanism |
| 2.05% | `cast_spell_8_step` | actions.rs | Spell resolution |

**String operations total: 8.89%** (is_contained_in 3.25% + write_str 2.15% + to_lowercase 1.49% + format 1.26% + others)

### Notable (1-2%)

- 1.90% - `GreedyManaResolver::check_payment` (mana algorithm)
- 1.80% - `drop_in_place<MtgError>` (error handling) - **ALREADY OPTIMIZED** (mtg-2: try_get() optimization, 10-13% speedup)
- 1.69% - `core::fmt::write` (formatting)
- 1.49% - `str::to_lowercase` (normalization) - **ALREADY CACHED** (mtg-2: CardCache)
- 1.46% - `get_valid_targets_for_spell` (targeting)
- 1.26% - `format_inner` (string formatting)
- 1.24% - `execute_step` (turn phases)
- 1.15% - `RandomController::choose_spell_ability` (AI)
- 1.10% - `cfree` (deallocation)
- 1.01% - `SmallVec::extend` (collection growth)
- 0.98% - `Formatter::pad_integral` (number formatting)
- 0.98% - `malloc` (allocation)
- 0.95% - `GameState::move_card` (zone transfers)
- 0.91% - `realloc` (reallocation)

## Cross-Reference: Perf vs Callgrind

### Agreement (Both identify as hot)

| Function | Perf % | Callgrind % | Conclusion |
|----------|--------|-------------|------------|
| ManaEngine::update | 15.00% | 30.5% | ✅ Confirmed hotspot |
| cast_spell_8_step | 2.05% | 23.3% | ⚠️ Perf lower (may be I/O bound in perf) |
| check_payment | 1.90% | 14.3% | ⚠️ Perf lower (algorithm varies by game state) |

### Perf-Only Hotspots (Not in Callgrind top 3)

- **priority_round (17.75%)** - Likely dominated by cache misses, not instruction count
- **String operations (8.89%)** - Memory-bound, not instruction-bound
- **bounds_check_payment (4.79%)** - Separate from check_payment proper

## Optimization Roadmap

### Phase 1: Priority Round Optimization (17.75% → ~10%)

**Target: 7-8% CPU reduction**

1. **Cache available actions** (OPT-NEW-1)
   - Invalidate on: card played, ability activated, phase change
   - Expected: 5% reduction (amortize action generation)
   - Risk: Medium (invalidation logic complexity)

2. **EntityStore Vec indices** (OPT-NEW-2)
   - Replace HashMap<EntityId, T> with Vec<Option<T>> + EntityId as index
   - Expected: 3% reduction (eliminate hash probes + comparisons)
   - Risk: Medium (memory overhead for sparse IDs)

3. **Pre-compute castability flags** (OPT-NEW-3)
   - Cache: has_activated_abilities, is_castable_spell per card
   - Expected: 2% reduction (avoid repeated type checks)
   - Risk: Low (simple caching, easy to invalidate)

### Phase 2: ManaEngine Optimization (15% → ~8%)

**Target: 7% CPU reduction**

**INCREMENTAL APPROACH (RECOMMENDED, per mtg-166):**

1. **ManaTracker incremental cache** (OPT-INC-1)
   - Cache mana sources, invalidate on battlefield changes
   - Expected: 5-7% reduction (amortize battlefield scan)
   - Risk: High (complex invalidation, undo/rewind integration)

**QUICK-WIN ALTERNATIVE:**

1. **Cache land type flags** (OPT-1, per mtg-166)
   - Pre-compute is_land, is_creature in CardCache
   - Expected: 4.5% reduction
   - Risk: Low (already have CardCache)

2. **Eliminate remaining Vec allocations** (OPT-MAN-1)
   - Pre-size all Vecs in update()
   - Expected: 2% reduction
   - Risk: Low (simple capacity hints)

### Phase 3: String Operation Optimization (8.89% → ~4%)

**Target: 4-5% CPU reduction**

1. **Mana color bitfield** (OPT-STR-1)
   - Replace string comparisons with bitwise ops
   - Expected: 2% reduction (eliminate is_contained_in overhead)
   - Risk: Low (already have precedent from Vec<ManaColor> work)
   - **Status: PARTIALLY DONE** - Vec<ManaColor> uses bitfield, but get_simple_mana_color still uses string comparisons (1.15%)

2. **String interning for ability text** (OPT-STR-2)
   - Intern common strings (ability oracle text)
   - Expected: 2% reduction (eliminate write_str + format)
   - Risk: Medium (global string pool management)

3. **Avoid to_lowercase in hot paths** (OPT-STR-3)
   - **Status: ALREADY DONE** (CardCache optimization, mtg-2)
   - Remaining 1.49% may be in other code paths

### Phase 4: Mana Payment Optimization (4.79% → ~2%)

**Target: 2-3% CPU reduction**

1. **Reuse candidates Vec** (OPT-7, per mtg-166)
   - Pre-allocate and clear instead of reallocate
   - Expected: 2% reduction
   - Risk: Low (simple buffer reuse)

2. **Algorithmic improvement** (OPT-PAY-1)
   - Investigate smarter payment ordering (reduce bounds checks)
   - Expected: 1% reduction
   - Risk: High (correctness risk)

## Total Potential Impact

**Conservative estimates:**

| Phase | Current % | Target % | Reduction | Difficulty |
|-------|-----------|----------|-----------|------------|
| Phase 1: priority_round | 17.75% | 10% | **7-8%** | Medium |
| Phase 2: ManaEngine | 15.00% | 8% | **7%** | High (incremental) / Low (quick-win) |
| Phase 3: String ops | 8.89% | 4% | **4-5%** | Medium |
| Phase 4: Mana payment | 4.79% | 2% | **2-3%** | Low |

**Total: 20-23% CPU reduction** (2,838 → 3,500+ games/sec)

## Implementation Priority

### Recommended Order (Risk vs Reward)

1. **OPT-1: Cache land type flags** (Low risk, 4.5% gain, 1-2 hours)
2. **OPT-7: Reuse candidates Vec** (Low risk, 2% gain, 30 min)
3. **OPT-STR-1: Mana color bitfield completion** (Low risk, 2% gain, 1 hour)
4. **OPT-NEW-3: Pre-compute castability** (Low risk, 2% gain, 2 hours)
5. **OPT-NEW-1: Cache available actions** (Medium risk, 5% gain, 1 day)
6. **OPT-INC-1: ManaTracker incremental** (High risk, 5-7% gain, 2-3 days)

**Quick wins (1-2 days work): 10-13% reduction**
**Full roadmap (1 week work): 20-23% reduction**

## Methodology

### Perf Command

```bash
perf record -F 997 -g --call-graph dwarf -o perf.data \
  -- ./target/release/rewind_bench -n 5000 -m sequential
```

### Analysis Command

```bash
perf report --stdio --no-children -n --sort symbol --percent-limit 0.5 -i perf.data
```

### Benchmark Configuration

- **Deck:** decks/old_school/03_robots_jesseisbak.dck (mirror match)
- **Mode:** Sequential rewind+replay (50% rewind point)
- **Allocator:** stats_alloc (glibc malloc with tracking)
- **Games:** 5000
- **Seed:** 43

### Hardware

- **CPU:** AMD Ryzen Threadripper PRO 7975WX (32 cores)
- **Kernel:** 6.15.6-200.fc42.x86_64
- **Perf version:** 6.17.1

## References

- **mtg-166:** Callgrind CPU hotspot analysis (instruction count view)
- **mtg-2:** Optimization tracking issue (allocation + CPU history)
- **ai_docs/cpu_hotspot_analysis_callgrind.md:** Callgrind detailed analysis
- **ai_docs/optimization_synthesis_cpu_and_allocation.md:** Combined roadmap
- **experiment_results/perf.data:** Raw profiling data (use `perf report -i`)

## Next Steps

1. Review this analysis with mtg-166 (Callgrind) to create unified roadmap
2. Implement quick wins (OPT-1, OPT-7, OPT-STR-1, OPT-NEW-3): 10-13% gain
3. Re-profile with perf + Callgrind to validate improvements
4. Decide: incremental caching (high risk/reward) vs continue quick wins

---

**Analysis Date:** 2025-11-29
**Commit:** a05ecc6 (perf profiling infrastructure working)
**Analyst:** Claude (AI) + User validation
