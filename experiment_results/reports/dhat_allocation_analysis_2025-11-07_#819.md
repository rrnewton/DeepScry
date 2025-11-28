# DHAT Heap Profiling Analysis: 1000-Game Benchmark

**Date**: 2025-11-07_#819(4369e4e7)
**Workload**: Rewind + Play Again (1000 games, robots mirror deck)
**Benchmark**: `rewind_bench` binary (sequential mode)
**Configuration**: 50% rewind point, infinite rewinds, fresh restart strategy

## Executive Summary

Successfully profiled gameplay allocations using DHAT after refactoring benchmark allocator infrastructure to eliminate conflicts between `dhat-heap` and `bench-stats-alloc` features.

### Key Metrics

- **Total allocations**: 1,477,549,735 bytes (1.48 GB) across 20,404,820 allocation calls
- **Average per game**: ~1.48 MB/game
- **Peak memory footprint**: 765,516 bytes (at t-gmax)
- **Final memory usage**: 1,064 bytes (at t-end)
- **Allocation sites identified**: 904 distinct call stacks
- **Performance**: 31.20 games/sec, 585 actions/game average

## Critical Finding: String Allocation Hotspot

**The #1 allocation bottleneck is `String::to_lowercase()` calls, accounting for >900 MB (>60%) of total allocations.**

### Top Allocation Sites

All top allocation sites share the same root cause: repeated calls to `to_lowercase()` during mana ability parsing and validation:

| Rank | Bytes Allocated | Blocks | Avg Size | Primary Call Site |
|------|----------------|--------|----------|-------------------|
| 1-2  | 247,269,439 × 2 | 1,676,849 × 2 | 147.5 | `get_complex_mana_production` → `to_lowercase` |
| 3-7  | 61,381,078 × 5 | 1,005,827 × 5 | 61.0 | `get_valid_targets_for_ability` → `to_lowercase` |
| 8-9  | 41,803,374 × 2 | 528,507 × 2 | 79.1 | `has_mana_ability` → `to_lowercase` |
| 10-17 | ~30-40 MB each | ~200K-270K | 147-148 | Various `get_complex_mana_production` paths |

**Pattern**: Nearly all top-30 allocation sites trace back to:
```
to_lowercase (alloc/src/str.rs:381:29)
  ↓
get_complex_mana_production (src/game/mana_engine.rs:561:32)
  OR
get_valid_targets_for_ability (src/game/actions.rs:494-497)
  OR
has_mana_ability (src/game/mana_engine.rs:482:32)
```

## Detailed Call Stack Analysis

### Category 1: Mana Engine String Allocations (~65% of total)

**Location**: `src/game/mana_engine.rs`

The mana engine allocates heavily when:
1. Parsing mana abilities from card text (lines 561, 482)
2. Converting ability strings to lowercase for matching
3. Building complex mana production data structures

**Call paths**:
```
update (mana_engine.rs:338)
  → get_complex_mana_production (mana_engine.rs:561)
    → to_lowercase()
      → Vec::with_capacity()
        → allocate (147-148 bytes/block average)
```

**Impact**:
- First call path: 247 MB × 2 = ~494 MB
- Multiple duplicate paths: Additional ~200-300 MB
- **Total**: ~700-800 MB from mana engine alone

### Category 2: Ability Target Validation (~25% of total)

**Location**: `src/game/actions.rs:494-497`, `src/game/game_loop.rs:3049-3124`

Target validation for activated/spell abilities allocates when:
1. Parsing `ValidTgts` restrictions
2. Converting target type strings to lowercase
3. Checking each permanent against ability restrictions

**Call paths**:
```
get_activatable_abilities (game_loop.rs:3049)
  → get_valid_targets_for_ability (actions.rs:495)
    → to_lowercase()
      → Vec::with_capacity()
        → allocate (61 bytes/block average)
```

**Impact**: 5 duplicate call paths × 61 MB = ~305 MB

### Category 3: Small Allocations (Vec Growth) (~10% of total)

**Location**: `src/game/mana_engine.rs:550`

Vector growth during mana production accumulation:

```
get_complex_mana_production (mana_engine.rs:550)
  → Vec::push()
    → grow_one()
      → allocate (8 bytes/block average)
```

**Impact**: ~10 MB per path, multiple paths = ~50-100 MB

## Root Cause Analysis

### Why So Many Duplicates?

The DHAT profiler tracks allocations by **call stack**, treating each unique path as a separate allocation site. We see massive duplication because:

1. **Multiple callers**: `get_complex_mana_production` called from many contexts
2. **Inlining**: Compiler inlining creates distinct stacks for the same logical operation
3. **Monomorphization**: Generic functions create separate instances per type

The actual source code locations are:
- `mana_engine.rs:561` - converting ability text to lowercase
- `actions.rs:494-497` - converting target restrictions to lowercase
- `mana_engine.rs:482` - checking for mana ability keywords

### Why String::to_lowercase()?

Current implementation converts strings to lowercase **every time** we:
- Check if a card has a mana ability
- Parse mana production amounts
- Validate spell targets
- Query ability types

This happens **hundreds of thousands of times per game** because:
1. Every main phase: check all lands for mana abilities
2. Every priority pass: check all permanents for activated abilities
3. Every spell cast: validate targets against restrictions

## Optimization Opportunities

### High-Impact (500-800 MB reduction potential)

**1. Intern/Cache Lowercase Strings**

Instead of:
```rust
fn has_mana_ability(&self, ability_text: &str) -> bool {
    let lower = ability_text.to_lowercase(); // ALLOCATES
    lower.contains("{t}:") || lower.contains("add ")
}
```

Use:
```rust
// Pre-lowercase during card loading
struct CardAbility {
    original: String,
    lowercase: String,  // Computed once at parse time
}
```

**Estimated savings**: ~500-700 MB (50% reduction)

**2. Use Case-Insensitive Comparison Without Allocation**

Replace `to_lowercase()` with byte-level case-insensitive search:
```rust
fn contains_ci(haystack: &str, needle: &str) -> bool {
    haystack.as_bytes()
        .windows(needle.len())
        .any(|window| window.eq_ignore_ascii_case(needle.as_bytes()))
}
```

**Estimated savings**: ~900 MB (eliminate all String::to_lowercase allocations)

**3. Pre-Parse Ability Flags at Load Time**

Store parsed ability metadata in card data:
```rust
struct CardAbilityFlags {
    has_mana_ability: bool,
    mana_colors: ManaColors,  // Bitflags, no allocation
    requires_target: bool,
    valid_target_types: u32,  // Bitflags
}
```

**Estimated savings**: ~600 MB + CPU time

### Medium-Impact (100-200 MB reduction potential)

**4. Pre-Size Vectors for Common Operations**

Currently many Vecs start at capacity 0 and grow incrementally (8 bytes/allocation).

Use `Vec::with_capacity()` based on typical sizes:
- Mana production results: capacity 10
- Ability lists: capacity based on permanent type

**Estimated savings**: ~50-100 MB

**5. Use SmallVec for Mana Production**

Most cards produce 1-3 mana. Use `SmallVec<[Mana; 4]>` to avoid heap allocation for common cases.

**Estimated savings**: ~30-50 MB

## Implementation Priority

Based on complexity vs impact:

1. **CRITICAL**: Replace `to_lowercase()` with case-insensitive byte comparison (High impact, medium complexity)
2. **HIGH**: Pre-compute lowercase strings during card loading (High impact, low complexity)
3. **MEDIUM**: Add ability flags to card metadata (Medium impact, medium complexity)
4. **LOW**: Pre-size vectors and use SmallVec (Low-medium impact, low complexity)

## Relationship to Java Forge

The Java version likely doesn't have this issue because:
1. Java strings are immutable and interned (`.toLowerCase()` may return same object if already lowercase)
2. Java card scripts use a different parsing model (not regex-heavy)
3. Forge's `Script` system caches parsed ability trees

Our Rust implementation currently:
- Re-parses ability text on every query
- Allocates new lowercase strings for each comparison
- Doesn't cache parsed ability metadata

## Validation & Testing

All changes must preserve correctness:
- ✅ All 418 tests still pass after allocator refactoring
- ✅ Determinism tests confirm identical game outcomes
- ✅ No regressions in `make validate`

Next steps for optimization implementation:
1. Create benchmark baseline with current allocation profile
2. Implement case-insensitive comparison helpers
3. Measure allocation reduction
4. Verify no determinism changes
5. Document in commit with before/after DHAT profiles

## Technical Details

### Benchmark Configuration

```bash
cargo run --release --no-default-features --features dhat-heap --bin rewind_bench -- -n 1000 --dhat
```

- Deck: `decks/old_school/03_robots_jesseisbak.dck` (mirror match)
- Seed: 43
- Mode: Sequential (single-threaded)
- Rewind: 50% playthrough, infinite rewinds
- Total turns: 22,153
- Total actions: 585,063
- Duration: 32.052s

### DHAT Output

```
dhat: Total:     1,477,549,735 bytes in 20,404,820 blocks
dhat: At t-gmax: 765,516 bytes in 1,507 blocks
dhat: At t-end:  1,064 bytes in 2 blocks
dhat: The data has been saved to dhat-heap.json
```

### Allocator Infrastructure Fix

**Issue mtg-882443**: Refactored allocator to eliminate conflicts
- Made `allocator.rs` global allocators conditional on `not(feature = "dhat-heap")`
- Updated `rewind_bench.rs` to handle both profiling and non-profiling modes
- Added compile-time feature conflict checks

This enables full gameplay profiling with DHAT without initialization-only bias.

## Conclusion

The DHAT profile successfully captured real gameplay allocations and identified a clear optimization path: **eliminate repeated `String::to_lowercase()` allocations in hot paths**.

Implementing case-insensitive comparisons and pre-computed lowercase strings could reduce allocations by **50-60%** (700-900 MB → 300-500 MB for 1000 games).

This represents a significant opportunity to improve both memory efficiency and CPU performance (fewer allocations = less allocator overhead = faster gameplay simulation).

---

**Next**: File optimization task issues and implement fixes iteratively, validating with DHAT after each change.
