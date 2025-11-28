# Profiling and Parallel Benchmark Analysis

**Date**: 2025-11-28
**Git Depth**: #952
**Commit**: abb8755c
**Machine**: 64-core (AMD/Intel x86_64), Ubuntu 25.10

## Executive Summary

This report characterizes the remaining allocation hotspots in the inner game loop and benchmarks parallel MCTS-style throughput with different allocators (mimalloc, jemalloc, stats_alloc).

**Key Findings**:
- Peak throughput: **71,978 games/sec** with mimalloc on 64 threads (33x parallel scaling)
- ManaEngine::update is the #1 CPU hotspot at **40%** of execution time
- String formatting still consumes **~18%** of CPU despite "Silent" verbosity
- mimalloc outperforms jemalloc by **27%** at 32 threads

---

## 1. DHAT Heap Profile Results

**Configuration**: 100 rewind+replay iterations from mid-game state
**Total allocations**: 1.23 MB in 26,906 blocks (~12.3 KB/replay)

### Top Allocation Sites (Inner Loop)

| Rank | Site | Bytes | % | Blocks | Avg Size |
|------|------|-------|---|--------|----------|
| 1 | `EntityStore::with_capacity` | 154 KB | 12.3% | 1 | 158 KB |
| 2 | Global allocator | 128 KB | 10.2% | 64 | 2 KB |
| 3 | `UndoLog::new` | 94 KB | 7.5% | 1 | 94 KB |
| 4 | `push_castable_spells` | 32 KB | 2.5% | 481 | 68 B |
| 5 | `ManaEngine::update` | 23 KB | 1.8% | 198 | 120 B |
| 6 | `get_available_spell_abilities` | 23 KB | 1.8% | 267 | 88 B |
| 7 | `GameState::advance_step` | 22 KB | 1.7% | 400 | 56 B |
| 8 | `push_castable_spells` (other) | 19 KB | 1.5% | 294 | 68 B |
| 9 | `push_castable_spells` (third) | 17 KB | 1.3% | 266 | 65 B |

**Observations**:
- One-time allocations (EntityStore, UndoLog) dominate at ~30% but aren't in the rewind cycle
- `push_castable_spells` and spell ability enumeration are the main inner-loop allocators
- Recent SmallVec optimizations have reduced per-action allocations to ~78 bytes/action

---

## 2. Callgrind CPU Profile Results

**Configuration**: 250 games, sequential mode, stats_alloc
**Total instructions**: 1.89 billion
**Cache performance**: Excellent (LL miss rate 0.0%)

### Top CPU Hotspots by Instruction Count

| Rank | Function | Instructions | % | Category |
|------|----------|--------------|---|----------|
| 1 | **`ManaEngine::update`** | 759M | **40.2%** | Mana tracking |
| 2 | `priority_round` (actions.rs) | 747M | 39.6% | Spell enumeration |
| 3 | `priority_round` (priority.rs) | 530M | 28.1% | Priority handling |
| 4 | `execute_step` (combat.rs) | 404M | 21.4% | Combat processing |
| 5 | `cast_spell_8_step` | 256M | 13.6% | Spell casting |
| 6 | `drop_in_place<MtgError>` | 190M | 10.1% | Error handling |
| 7 | `core::fmt::write` | 181M | 9.6% | String formatting |
| 8 | `format_inner` | 162M | 8.6% | String allocation |
| 9 | `tap_for_mana_for_cost` | 149M | 7.9% | Mana payment |
| 10 | `GreedyManaResolver::check_payment` | 113M | 6.0% | Mana validation |
| 11 | `choose_spell_ability_to_play` | 110M | 5.8% | AI decision |
| 12 | `GameState::undo` | 56M | 3.0% | Undo operations |
| 13 | `get_valid_targets_for_spell` | 48M | 2.5% | Targeting |
| 14 | `realloc` (libc) | 46M | 2.4% | Memory allocation |

### Cache Statistics

| Metric | Value |
|--------|-------|
| I1 miss rate | 0.75% |
| D1 miss rate | 0.6% |
| LL miss rate | 0.0% |

**Observations**:
- `ManaEngine::update` alone is 40% - prime optimization target
- String formatting (`fmt::write` + `format_inner`) is ~18% despite Silent mode
- Priority round functions combined account for ~68% of execution
- Cache behavior is excellent - CPU bound, not memory bound

---

## 3. Parallel Benchmark Results

### Parallel Scaling with mimalloc (64-core machine)

| Threads | Time/game | Games/sec | Scaling vs 1T |
|---------|-----------|-----------|---------------|
| 1 | 458 µs | 2,173 | 1.0x |
| 2 | 230 µs | 4,387 | 2.0x |
| 4 | 117 µs | 8,716 | 4.0x |
| 8 | 60 µs | 16,711 | 7.7x |
| 16 | 31 µs | 32,533 | 15.0x |
| 32 | 18 µs | 59,261 | 27.3x |
| 48 | 16 µs | 63,603 | 29.3x |
| 64 | 14 µs | **71,978** | **33.1x** |

### Allocator Comparison (Games/sec)

| Threads | mimalloc | jemalloc | stats_alloc | mimalloc advantage |
|---------|----------|----------|-------------|-------------------|
| 1 | 2,173 | 2,147 | 2,127 | +1.2% vs jemalloc |
| 8 | 16,711 | 16,385 | 5,791 | +2.0% vs jemalloc |
| 32 | 59,261 | 46,588 | N/A | **+27.2%** vs jemalloc |
| 64 | 71,978 | 66,004 | N/A | +9.1% vs jemalloc |

### Pinned vs Rayon Parallel (mimalloc)

| Threads | ParRayon | ParPinned |
|---------|----------|-----------|
| 8 | 16,711/s | 16,751/s |
| 32 | 59,261/s | 59,254/s |
| 64 | 71,978/s | 58,114/s |

**Observations**:
- mimalloc wins at high thread counts, especially 32 threads (+27%)
- ParRayon slightly outperforms ParPinned at 64 threads
- stats_alloc tracking introduces ~3x slowdown in parallel due to synchronization

### Allocation Rates

| Configuration | Bytes/action | Bytes/game |
|---------------|--------------|------------|
| Sequential | 77.6 | 114 KB |
| 8 threads parallel | 597.7 | 880 KB |

The parallel benchmark allocates ~7.7x more per action due to game state cloning for each thread.

---

## 4. Optimization Recommendations

### High Priority

1. **ManaEngine::update caching (40% CPU)**
   - Cache mana source calculations when board state unchanged
   - Invalidate on permanent enter/leave battlefield, tap/untap events

2. **String formatting elimination (18% CPU)**
   - Ensure `log_if_verbose!` macro prevents format string construction
   - Use `#[cfg(feature = "verbose-logging")]` more aggressively

3. **Priority round spell enumeration caching**
   - Cache available spells when board unchanged between priority passes
   - Invalidate on state-based actions or spell resolution

### Medium Priority

4. **Reduce clone overhead for parallel**
   - Current: ~114 KB per game state clone
   - Consider structural sharing or copy-on-write for large collections

5. **check_payment/tap_for_mana_for_cost (~14%)**
   - Profile mana payment inner loop
   - Consider memoization for repeated payment attempts

---

## 5. Recent Allocation Optimizations (commits #940-952)

SmallVec conversions completed:
- `mill_cards` return type
- attacker/blocker creature lists
- `mana_payment` and conditional effect cloning
- combat `get_attackers/get_blockers_list`
- `heuristic_controller` combat simulation
- mana payment candidate sources
- `random_controller` shuffles
- `get_lands_in_hand` and spell/ability queries

**Result**: Reduced bytes/action from ~228 to ~78 (66% reduction)

---

## Appendix: Raw Callgrind Cache Statistics

```
I   refs:      1,888,166,638
I1  misses:       14,080,287
LLi misses:            8,379
I1  miss rate:          0.75%
LLi miss rate:          0.00%

D   refs:        795,649,818  (485,964,478 rd + 309,685,340 wr)
D1  misses:        4,968,080  (  3,906,892 rd +   1,061,188 wr)
LLd misses:           53,325  (     18,233 rd +      35,092 wr)
D1  miss rate:           0.6% (        0.8%   +         0.3%  )
LLd miss rate:           0.0% (        0.0%   +         0.0%  )
```
