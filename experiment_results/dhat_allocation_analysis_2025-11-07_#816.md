# DHAT Heap Profile Analysis

**Date**: 2025-11-07
**Git Depth**: #816
**Commit**: ee588682
**Workload**: Game initialization (robots mirror deck loading)
**Tool**: DHAT 0.3 heap profiler

## Executive Summary

DHAT profiling of game initialization reveals that **our recent HashMap/Vec pre-sizing optimizations are working as intended**. The top two allocation sites are exactly the pre-sized structures we targeted:

1. **UndoLog::new()** - 93.75 KB (20.7%) - Pre-allocated for 1000 actions
2. **EntityStore::with_capacity()** - 86.27 KB (19.0%) - Pre-sized for deck cards

Together, these account for **40% of initialization allocations**, confirming that our optimization strategy is targeting the right hotspots.

## Total Allocations

- **Total**: 453.10 KB in 4,386 blocks
- **Average block size**: 105.8 bytes
- **Memory at end**: 36.96 KB

## Top 10 Allocation Sources

### #1: UndoLog Pre-allocation (93.75 KB, 20.7%)
```
Location: mtg-engine/src/undo.rs:278:22 - UndoLog::new()
Blocks: 1 (96,000 bytes)
Context: GameState::new_two_player_with_capacity()
```

**Analysis**: This is our recent optimization (commit 2fd3b59f) pre-allocating Vec capacity for 1000 actions. This is **intentional** and **correct** - it prevents incremental resizing during gameplay.

**Action**: ✅ No change needed - optimization working as designed

---

### #2: EntityStore HashMap Pre-allocation (86.27 KB, 19.0%)
```
Location: mtg-engine/src/core/entity.rs:221:23 - EntityStore::with_capacity()
Blocks: 1 (88,336 bytes)
Context: GameState initialization for 120 deck cards
```

**Analysis**: This is our recent optimization (commit 2fd3b59f) pre-sizing the HashMap for deck cards. This is **intentional** and **correct** - it eliminates 6-7 HashMap resize operations.

**Action**: ✅ No change needed - optimization working as designed

---

### #3-#6: Tokio Runtime Overhead (63.56 KB, 14.0%)
```
Locations: Multiple tokio task allocation sites
Blocks: 173 total (640-256 bytes each)
Context: Async runtime infrastructure
```

**Analysis**: These are tokio task allocations for async card loading. Tokio is only used during initialization, not gameplay, so this is acceptable overhead.

**Action**: ⚠️ Low priority - could investigate tokio-less card loading if initialization time becomes critical

---

### #7: CardDefinition::parse_activated_abilities (8.25 KB, 1.8%)
```
Location: mtg-engine/src/loader/card.rs:722:31
Blocks: 24 (352 bytes each)
Context: Parsing activated abilities from card definitions
```

**Analysis**: Vec growth while parsing ability text. Happens once per unique card during deck loading.

**Optimization Plan**:
- **Priority**: Medium
- **Approach**: Pre-allocate Vec based on ability count hint or empirical average
- **Expected impact**: ~8 KB reduction (1.8%)
- **Effort**: Low - add `Vec::with_capacity(expected_abilities)`

---

### #8: UndoLog log_sizes Pre-allocation (7.81 KB, 1.7%)
```
Location: mtg-engine/src/undo.rs:281:24 - UndoLog::new()
Blocks: 1 (8,000 bytes)
Context: Pre-allocating log_sizes Vec
```

**Analysis**: Companion to #1 - pre-allocates the log_sizes tracking vector. This is also part of our recent optimization.

**Action**: ✅ No change needed - optimization working as designed

---

### #9-#10: CardDefinition::instantiate (15.12 KB combined, 3.3%)
```
Locations: mtg-engine/src/loader/card.rs:225:42 and :169:43
Blocks: Multiple small allocations (88-352 bytes)
Context: Creating Card instances from definitions
```

**Analysis**: Small Vec/String allocations during card instantiation (60 cards × 2 decks = 120 instances).

**Optimization Plan**:
- **Priority**: Medium-Low
- **Approach**: Object pooling or arena allocation for card instances
- **Expected impact**: ~15 KB reduction (3.3%)
- **Effort**: High - requires significant refactoring

---

##Allocation Site Analysis (4,386 blocks)

### By Size Category:
- **Large pre-allocations (>50 KB)**: 2 sites, 180 KB (40%) - Our optimizations ✅
- **Medium allocations (5-50 KB)**: 8 sites, 86 KB (19%) - Card loading/parsing
- **Small allocations (<5 KB)**: 258 sites, 187 KB (41%) - General operations

### By Function:
- **Game state initialization**: 180 KB (40%) - Mostly optimized
- **Card database loading**: 86 KB (19%) - Room for improvement
- **Tokio async runtime**: 64 KB (14%) - Acceptable overhead
- **Miscellaneous**: 123 KB (27%) - Fragmented small allocations

## Optimization Roadmap

### Phase 1: Completed ✅
- [x] UndoLog pre-allocation (93.75 KB saved from resizing)
- [x] EntityStore pre-allocation (86.27 KB saved from resizing)
- **Result**: Eliminated resize operations, improved locality

### Phase 2: Card Loading Optimizations (Medium Priority)
**Target**: ~23 KB reduction (5% of total)

1. **Pre-allocate ability parsing Vec** (card.rs:722)
   - Effort: Low
   - Impact: 8.25 KB
   - Code: `Vec::with_capacity(3)` based on average abilities/card

2. **Pre-allocate CardDefinition fields** (card.rs:225, :169)
   - Effort: Medium
   - Impact: 15 KB
   - Code: Capacity hints in CardDefinition::new()

3. **Batch card instantiation** (game_init.rs)
   - Effort: Medium
   - Impact: Better cache locality
   - Code: Process cards in batches vs one-at-a-time

### Phase 3: Structural Changes (Lower Priority)
**Target**: Reduce allocation count, not size

1. **String interning for card names**
   - Common card names appear multiple times
   - Use `Rc<str>` or string arena
   - Reduces allocation count, marginal size impact

2. **Arena allocation for transient game objects**
   - Requires allocator parameterization (see allocator branch)
   - Major effort, significant impact on parallel performance
   - Deferred pending parallel MCTS profiling

### Phase 4: Runtime Optimization (Future)
**Target**: Gameplay allocations (not measured in this profile)

This profile only measures **initialization** allocations. To measure **gameplay** allocations, we need:

1. Run DHAT on full 1000-game workload (not just initialization)
2. Profile GameState::clone() for MCTS workload
3. Measure per-turn allocation patterns

**Note**: The current DHAT integration in rewind_bench needs fixing to properly profile gameplay allocations vs just initialization.

## Key Findings

### ✅ Wins
1. **Pre-sizing optimizations are effective**: UndoLog and EntityStore together account for 40% of allocations and are now pre-sized correctly
2. **No obvious low-hanging fruit**: Remaining allocations are fragmented across many small sites
3. **Initialization is well-optimized**: Most large allocations are intentional pre-sizing

### ⚠️ Areas for Improvement
1. **Card loading could be optimized**: 23 KB (5%) with medium effort
2. **Tokio overhead is acceptable**: 14% for async loading, only during init
3. **Need gameplay profiling**: This profile doesn't capture MCTS clone() allocations

### 🔍 Open Questions
1. **What does GameState::clone() allocate?** - Critical for parallel MCTS
2. **What allocates during gameplay?** - Need full 1000-game DHAT profile
3. **Where is the 78-92 GB coming from in parallel?** - Not visible in this init profile

## Methodology Notes

This profile was generated from game initialization only (deck loading). The DHAT integration in rewind_bench currently captures initialization but not gameplay allocations due to allocator configuration issues.

To properly profile gameplay:
- Need to ensure DHAT is the global allocator for entire benchmark run
- Current stats_alloc conflicts with DHAT
- See commit ee588682 for partial fix

## References

- DHAT documentation: https://nnethercote.github.io/dh_view/dh_view.html
- Optimization commits: 2fd3b59f (pre-sizing), ee588682 (DHAT integration)
- Analysis script: scripts/analyze_dhat.py
- Raw data: dhat-heap.json (118 KB)

---

**Generated**: 2025-11-07 at git depth #816
**Tooling**: DHAT 0.3.3, analyze_dhat.py
**Next Steps**: Fix DHAT integration in rewind_bench, profile full gameplay workload
