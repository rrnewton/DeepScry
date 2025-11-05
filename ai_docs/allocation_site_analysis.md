# Allocation Site Analysis for Allocator API Migration

**Date**: 2025-11-05
**Branch**: allocator
**Related Issue**: mtg-151

## Executive Summary

This document inventories all allocation sites in `mtg-engine/src/` and categorizes
types by their compatibility with Rust's Allocator API. This analysis informs the
preparatory refactorings needed before full allocator parameterization.

**Key Finding**: SmallVec (189 occurrences) is the major blocker, as it lacks
allocator API support but is critical for inline optimization performance.

## Methodology

Analyzed all `.rs` files in `mtg-engine/src/` using grep and manual inspection:

```bash
cd mtg-engine/src
grep -r "HashMap\|Vec<\|String\|SmallVec\|Arc<\|Rc<\|Box<" --include="*.rs"
```

## Type Categories

### ✅ Types WITH Allocator API Support

These types can be parameterized immediately once core infrastructure is ready.

#### Vec<T, A>

**Status**: Full allocator API support in nightly Rust
**Occurrences**: 254+ total (includes HashMap internal Vecs)

**Usage Pattern**:
```rust
// Before:
pub struct GameState {
    pub cards: Vec<Card>,
}

// After:
pub struct GameState<A: Allocator = Global> {
    pub cards: Vec<Card, A>,
    allocator: A,
}
```

**Action**: Parameterize all Vec fields in core types (Phase 2)

---

#### HashMap<K, V, S, A> / BTreeMap<K, V, A>

**Status**: Full allocator API support in nightly Rust
**Occurrences**: Included in 254 count above

**Usage Pattern**:
```rust
// Before:
pub zones: HashMap<Zone, Vec<CardId>>,

// After:
pub zones: HashMap<Zone, Vec<CardId, A>, RandomState, A>,
```

**Action**: Parameterize all HashMap/BTreeMap fields (Phase 2)

---

#### Box<T, A>

**Status**: Full allocator API support in nightly Rust
**Occurrences**: 12

**Locations**:
- Error type boxing
- Trait object boxing (dyn Trait)
- Recursive type indirection

**Action**: Verify nightly support, parameterize if needed (mtg-154)

---

### ⚠️ Types WITHOUT Allocator API Support

These types are blockers and require workarounds or acceptance.

#### SmallVec<T> - MAJOR BLOCKER

**Status**: NO allocator API support
**Occurrences**: 189 across 19 files
**Related Issue**: mtg-153

**Why It Matters**:
- Critical for inline optimization (avoids heap allocation for small collections)
- Heavily used in hot paths (counters, abilities, zones)
- Performance-critical trade-off

**Distribution**:
```
19 files total:
- game/heuristic_controller.rs (many)
- game/actions.rs (many)
- core/card.rs (counters, abilities)
- game/state.rs (zones)
```

**Example Usage**:
```rust
pub struct Card {
    pub counters: SmallMap<CounterType, u32, 4>,  // SmallMap uses SmallVec internally
    pub activated_abilities: SmallVec<[ActivatedAbility; 2]>,
    pub subtypes: SmallVec<[Subtype; 2]>,
}
```

**Options**:

**Option A: Keep SmallVec as-is** (RECOMMENDED)
- **Pros**:
  - Zero code changes
  - Maintains inline optimization
  - Most SmallVecs stay inline (no heap allocation)
- **Cons**:
  - Overflow allocations use global allocator
  - Some parallel contention remains (likely minimal)
- **Risk**: Low
- **Effort**: None
- **Decision Point**: Measure parallel performance with per-game allocators
  - If efficiency reaches 60%+: Accept SmallVec as-is
  - If efficiency plateaus <60%: Revisit options B or C

**Option B: Fork SmallVec with allocator support** (AMBITIOUS)
- **Pros**:
  - Complete allocator coverage
  - Maintains inline optimization
  - Future-proof for upstream contribution
- **Cons**:
  - Significant engineering effort (1-2 weeks)
  - Maintenance burden (track upstream changes)
  - May not be accepted upstream
- **Risk**: Medium
- **Effort**: High
- **Decision Point**: Only if Option A proves insufficient

**Option C: Replace with Vec<T, A>** (SIMPLE)
- **Pros**:
  - Full allocator support
  - Simple refactoring
- **Cons**:
  - Lose inline optimization
  - Always heap-allocates
  - May regress sequential performance
- **Risk**: High (performance regression)
- **Effort**: Medium
- **Decision Point**: Benchmark required before committing

**Recommendation**: **Start with Option A**
- Implement per-game allocators with SmallVec unchanged
- Measure parallel efficiency improvement
- If efficiency reaches 60-70%, accept SmallVec limitation
- If efficiency plateaus below 60%, revisit Option B

---

#### String - MEDIUM BLOCKER

**Status**: NO allocator API support
**Occurrences**: 185 (after filtering CardName/PlayerName)
**Related Issue**: mtg-152

**Why It Matters**:
- Common type for text data
- Some hot-path usage (card names, descriptions)
- Most critical uses already Arc<str> (CardName, PlayerName)

**Distribution by Category**:

1. **Arc<str> (Already Optimized)** - ~60%
   - CardName: Arc<str>
   - PlayerName: Arc<str>
   - Subtype: Arc<str>
   - **Action**: None needed ✅

2. **Temporary Strings (Acceptable)** - ~30%
   - Error messages
   - Debug/logging output
   - UI display text
   - **Action**: Accept as-is ✅

3. **Hot-Path Strings (Needs Audit)** - ~10%
   - Card descriptions (if cloned frequently)
   - Parser intermediate results
   - Controller state strings
   - **Action**: Audit and potentially convert to Arc<str>

**Action Plan (mtg-152)**:
1. Audit remaining 185 String usages
2. Categorize: Arc<str> candidates vs acceptable String
3. Convert hot-path Strings to Arc<str> if beneficial
4. Accept remaining Strings (mostly TUI/error handling)

**Estimated Impact**:
- Low: Most critical strings already Arc<str>
- Likely < 5% of parallel overhead

---

#### Arc<T> / Rc<T> - MINOR ISSUE

**Status**: NO allocator API support (shared ownership)
**Occurrences**: 6

**Why It Matters**:
- Shared ownership prevents custom allocators
- Mostly used for immutable shared data (acceptable)

**Locations**:
```
- CardDefinition storage (Arc<CardDefinition>)
- Shared card database references
- Thread-safe shared state
```

**Action**: Accept as-is ✅
- Arc/Rc are for shared immutable data
- Custom allocators don't make sense for shared ownership
- Minimal performance impact

---

## Allocation Density by File

Files ranked by number of allocation sites (Vec, HashMap, String):

| Rank | File | Sites | Category | Priority |
|------|------|-------|----------|----------|
| 1 | `game/fancy_tui_controller.rs` | 66 | TUI | Low (not hot path) |
| 2 | `game/heuristic_controller.rs` | 57 | AI | **HIGH** (critical hot path) |
| 3 | `game/actions.rs` | 32 | Core | **HIGH** (critical hot path) |
| 4 | `main.rs` | 26 | CLI | Low (startup only) |
| 5 | `game/interactive_controller.rs` | 23 | TUI | Low (not hot path) |
| 6 | `game/game_loop.rs` | 23 | Core | **HIGH** (critical hot path) |
| 7 | `game/controller.rs` | 22 | Core | Medium |
| 8 | `puzzle/state.rs` | 21 | Puzzle | Low |
| 9 | `puzzle/card_notation.rs` | 14 | Puzzle | Low |
| 10 | `game/replay_controller.rs` | 14 | Replay | Low |

**Critical Files for Phase 2 Parameterization**:
1. `game/state.rs` - GameState struct
2. `game/game_loop.rs` - Game execution
3. `game/actions.rs` - Game actions
4. `game/heuristic_controller.rs` - AI decision-making
5. `game/mana_engine.rs` - Mana calculations
6. `core/card.rs` - Card data structures

## SmallVec Deep Dive

SmallVec is used in the following patterns:

### Pattern 1: Card Counters (Inline: 4)

```rust
pub counters: SmallMap<CounterType, u32, 4>,
```

**Rationale**: Most creatures have 0-2 counter types
**Heap Impact**: Minimal (rarely exceeds 4)
**Allocator Impact**: Very low (inline storage dominates)

### Pattern 2: Activated Abilities (Inline: 2)

```rust
pub activated_abilities: SmallVec<[ActivatedAbility; 2]>,
```

**Rationale**: Most permanents have 0-1 activated abilities
**Heap Impact**: Low (rarely exceeds 2)
**Allocator Impact**: Low

### Pattern 3: Subtypes (Inline: 2)

```rust
pub subtypes: SmallVec<[Subtype; 2]>,
```

**Rationale**: Most creatures have 1-2 subtypes
**Heap Impact**: Low
**Allocator Impact**: Low

### Pattern 4: Temporary Collections

```rust
let mut targets = SmallVec::<[CardId; 8]>::new();
```

**Rationale**: Temporary calculation buffers
**Heap Impact**: Medium (size varies)
**Allocator Impact**: Medium (could benefit from per-turn arena)

**Analysis**: Patterns 1-3 are optimal with SmallVec (inline storage wins).
Pattern 4 could potentially use Vec<T, A> with per-turn allocator instead,
but would need benchmarking to validate.

## Preparatory Refactoring Recommendations

### High Priority (Before Phase 2)

1. **mtg-152: Audit String usage** (1 week)
   - Categorize 185 String occurrences
   - Convert hot-path Strings to Arc<str> if beneficial
   - Accept remaining Strings

2. **mtg-153: SmallVec strategy decision** (1 week)
   - Implement per-game allocators with SmallVec unchanged (Option A)
   - Benchmark parallel efficiency
   - Decision: Accept or escalate to Option B/C

3. **mtg-154: Verify Box<T, A> support** (1 day)
   - Test Box<T, BumpAllocator> compilation
   - Document any limitations
   - Plan parameterization if supported

### Medium Priority (During Phase 2)

4. **Monitor SmallVec impact**
   - Track allocation counts for SmallVec overflow
   - Measure parallel contention contribution
   - Re-evaluate if efficiency plateaus

5. **Profile String allocations**
   - Identify any String hot spots
   - Convert to Arc<str> if found
   - Document acceptable String usage

### Low Priority (Phase 4+)

6. **Per-turn temporary buffers**
   - Identify SmallVec pattern 4 usage (temporary collections)
   - Consider Vec<T, A> with per-turn allocator
   - Benchmark before/after

## Expected Allocation Breakdown

After per-game allocator implementation, expected allocation sources:

| Source | Percentage | Allocator | Notes |
|--------|------------|-----------|-------|
| Vec<T, A> in GameState | 60% | Per-game | ✅ Fully controlled |
| HashMap<K,V,S,A> in GameState | 20% | Per-game | ✅ Fully controlled |
| SmallVec overflow | 10% | Global | ⚠️ Acceptable (rare overflow) |
| String (non-Arc) | 5% | Global | ✅ Acceptable (mostly UI/errors) |
| Arc/Rc allocations | 5% | Global | ✅ Acceptable (shared data) |

**Total allocator coverage**: 80% (Vec + HashMap)
**Remaining global allocation**: 20% (SmallVec overflow + String + Arc/Rc)

**Analysis**: 80% coverage is excellent and should achieve 60-70% parallel
efficiency target. The 20% global allocation is mostly rare events or shared
data, unlikely to cause significant contention.

## Risk Assessment

### Low Risk ✅

- **Arc/Rc without allocator support**: Acceptable (shared data)
- **String for UI/errors**: Acceptable (not hot path)
- **SmallVec with inline storage**: Low contention (rarely overflows)

### Medium Risk ⚠️

- **SmallVec overflow to global allocator**: Could cause contention
  - **Mitigation**: Monitor with profiling, escalate to Option B if needed
- **Hot-path String allocations**: Could add overhead
  - **Mitigation**: Audit (mtg-152), convert to Arc<str> if found

### High Risk ❌

- **Replacing SmallVec with Vec<T, A>**: Performance regression
  - **Mitigation**: Only if Option A insufficient, benchmark required
- **Complex lifetime management**: Code complexity explosion
  - **Mitigation**: Use default A = Global, provide helpers

## Success Metrics

### Phase 1 (Preparatory Refactorings)

- ✅ All String usage audited and categorized
- ✅ SmallVec strategy decision documented
- ✅ Box<T, A> support verified
- ✅ No performance regressions from preparatory changes

### Phase 2 (Core Parameterization)

- ✅ GameState<A>, UndoLog<A>, Player<A>, ManaEngine<A> compile
- ✅ Existing code works with A = Global
- ✅ All tests pass
- ✅ Sequential performance within 2% of baseline

### Phase 3 (Per-Game Allocators)

- ✅ Parallel efficiency improves from 5.6% to 50-60%
- ✅ Allocator contention eliminated (profiling confirms)
- ✅ Both System and Bumpalo allocators work

### Phase 4 (Per-Turn Allocators)

- ✅ Per-turn allocation approaches zero
- ✅ Parallel efficiency improves to 70-80%
- ✅ Aggregate throughput exceeds 1M games/sec

## Conclusion

**Allocator API migration is viable with 80% coverage**.

The SmallVec limitation (10% of allocations) is acceptable given:
1. Inline storage dominates (rarely heap-allocates)
2. Overflow is rare and unlikely to cause significant contention
3. Alternative options (B/C) have high risk/effort

**Recommended Path**:
1. Proceed with Phase 1 (audit String, decide SmallVec strategy)
2. Implement Phase 2 (parameterize core types)
3. Measure Phase 3 (per-game allocators)
4. Re-evaluate SmallVec if efficiency plateaus below 60%

**Expected Outcome**: 70-80% parallel efficiency, meeting MCTS requirements.

---

**Next Steps**: Create mtg-152, mtg-153, mtg-154 sub-issues
**Last Updated**: 2025-11-05 (Branch: allocator, Commit: ef8d3f00)
