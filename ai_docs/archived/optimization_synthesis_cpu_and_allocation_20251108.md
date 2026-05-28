# Optimization Synthesis: CPU + Allocation Profiling

**Date**: 2025-11-08_#831
**Workload**: Rewind + Play Again (robots mirror deck)
**Profiling Tools**: Valgrind Callgrind (CPU) + DHAT (allocations)

## Executive Summary

By combining CPU profiling (Callgrind) and allocation profiling (DHAT), we've identified a clear optimization roadmap. The **CardCache optimization (#822) already eliminated 94.2% of allocations**, but CPU profiling reveals the **remaining performance bottlenecks are in Vec operations**, not string allocations.

### Key Insight: CPU ≠ Allocations

The top CPU hotspots and top allocation sites are **different but related**:

| Category | CPU Hotspot | Allocation Hotspot | Relationship |
|----------|-------------|-------------------|--------------|
| **Mana System** | `ManaEngine::update` (30.5% CPU) | `mana_engine.rs:550` (27 MB Vec growth) | Same system, different operations |
| **Spell Casting** | `cast_spell_8_step` (23.3% CPU) | `actions.rs` (1.71 MB) | Error recovery overhead |
| **Mana Payment** | `check_payment` (14.3% CPU) | `mana_payment.rs:580` (2.57 MB) | O(n²) lookups + Vec allocations |

**The pattern**: High CPU usage correlates with Vec operations (growth, iteration, lookup), but most allocations come from **repetitive small Vecs** in hot paths.

## Profiling Results Comparison

### Callgrind CPU Profile (250 games)

```
Total instructions: 18.8 billion
Duration: 1-2 minutes
```

**Top 3 hotspots (68% of total CPU)**:
1. `ManaEngine::update`: 5.7B instructions (30.5%)
2. `cast_spell_8_step`: 4.4B instructions (23.3%)
3. `GreedyManaResolver::check_payment`: 2.7B instructions (14.3%)

### DHAT Allocation Profile (1000 games)

```
Total allocations: 86.4 MB (post-cache optimization)
Total blocks: 5.6M
Duration: 9.15 seconds
Throughput: 109.29 games/sec
```

**Top allocation site**:
- `mana_engine.rs:550`: 27 MB (31% of total) - Vec growth for dual land colors

**Allocation breakdown**:
- Vec growth (amortized): 53.89 MB (65.4%)
- Vec with_capacity: 26.18 MB (31.8%)
- Other: 2.32 MB (2.8%)

## Cross-Reference Analysis

### Finding 1: ManaEngine is both CPU and allocation intensive

**CPU**: 30.5% of instructions (5.7B)
- String comparisons for land names (now cached ✅)
- Subtype iteration for dual lands
- Vec iteration over permanents

**Allocations**: 27 MB (31% of remaining allocations)
- Vec growth in `colors.push(c)` at line 550
- Called once per mana source per mana engine update

**Optimization OPT-1 (from CPU analysis)**: Cache land types in CardCache
- **CPU impact**: 4.5% reduction
- **Allocation impact**: Unknown (probably minimal - flags are boolean)

**Optimization DHAT-1 (from allocation analysis)**: Pre-size Vec at line 550
- **CPU impact**: Unknown (fewer reallocations = less CPU)
- **Allocation impact**: 15-20 MB reduction (27 MB → ~7-12 MB)

**COMBINED**: Both optimizations target `ManaEngine::update` and are complementary!

### Finding 2: Spell casting has different bottlenecks

**CPU**: 23.3% of instructions (4.4B)
- Three error recovery paths with duplicated logic
- Repeated state restoration
- mana_engine.update() calls (30+ times per spell)

**Allocations**: 1.71 MB (combat) + some in game_loop
- Vec allocations for sources_to_tap
- Action queue growth

**Optimization OPT-4 (from CPU analysis)**: Pre-validate before unwinding
- **CPU impact**: 1.2% reduction
- **Allocation impact**: Potentially reduces some error recovery allocations

**Optimization DHAT-2 (from allocation analysis)**: Pre-size action queue Vecs
- **CPU impact**: Unknown (fewer reallocations)
- **Allocation impact**: 5-10 MB reduction

**COMBINED**: Pre-validation helps CPU, pre-sizing helps allocations

### Finding 3: Mana payment resolver is allocation-heavy

**CPU**: 14.3% of instructions (2.7B)
- Greedy algorithm: 5 iterations (one per color)
- `Vec::contains()` for tap order lookups: O(n²)
- Score computation per source

**Allocations**: 2.57 MB
- Vec allocation for candidates (once per color = 5x per spell)
- Already using `with_capacity` but possibly oversized

**Optimization OPT-7 (from CPU analysis)**: Reuse candidates Vec
- **CPU impact**: 4.3% reduction (less allocator overhead)
- **Allocation impact**: ~2 MB reduction (2.57 MB → ~0.5 MB)

**Optimization OPT-8 (from CPU analysis)**: Use HashSet for tap_order
- **CPU impact**: 5.0% reduction (O(n²) → O(n))
- **Allocation impact**: Minimal (one HashSet vs repeated Vec operations)

**COMBINED**: Vec reuse helps both CPU and allocations massively

## Unified Optimization Roadmap

Based on combined analysis, here's the prioritized roadmap:

### Phase 1: Quick Wins (High Impact, Low Effort)

**Total estimated impact**: 11-15% CPU reduction + 20-25 MB allocation reduction

| ID | Optimization | CPU Reduction | Allocation Reduction | Effort | File |
|----|--------------|---------------|---------------------|--------|------|
| **OPT-1** | Cache land types in CardCache | **4.5%** | Minimal | 1-2 hours | `mana_engine.rs:254-353` |
| **OPT-7** | Reuse candidates Vec | **4.3%** | **~2 MB** | 30 min | `mana_payment.rs:246-390` |
| **DHAT-1** | Pre-size Vec at mana_engine.rs:550 | 0.5-1.0% | **15-20 MB** | 5 min | `mana_engine.rs:550` |

**Implementation order**: DHAT-1 → OPT-7 → OPT-1 (easiest to hardest)

### Phase 2: Medium Impact (Good ROI)

**Total estimated impact**: 5-8% CPU reduction + 10-15 MB allocation reduction

| ID | Optimization | CPU Reduction | Allocation Reduction | Effort | File |
|----|--------------|---------------|---------------------|--------|------|
| **OPT-8** | HashSet for tap_order lookup | **5.0%** | Minimal | 1 hour | `mana_payment.rs:246-390` |
| **DHAT-2** | Pre-size Vecs in game_loop | 0.5-1.0% | **5-10 MB** | 15 min | `game_loop.rs:1413,3111` |
| **DHAT-3** | SmallVec for dual land colors | 0.5-1.0% | **8-12 MB** | 30 min | `mana_engine.rs:550` |

**Implementation order**: DHAT-2 → OPT-8 → DHAT-3

### Phase 3: Structural Changes (Lower ROI, Higher Complexity)

**Total estimated impact**: 2-4% CPU reduction + 5-10 MB allocation reduction

| ID | Optimization | CPU Reduction | Allocation Reduction | Effort | File |
|----|--------------|---------------|---------------------|--------|------|
| **OPT-2** | Cache color counts in ManaPool | **1.5%** | Minimal | 2-3 hours | `mana_types.rs` |
| **OPT-4** | Pre-validate before unwinding | **1.2%** | ~1 MB | 2-3 hours | `actions.rs:640-771` |
| **DHAT-4** | Vec pooling in random controller | Minimal | **4-5 MB** | 1 hour | `random_controller.rs:103` |
| **DHAT-5** | Snapshot buffer pooling | Minimal | **2-5 MB** | 2+ hours | `state.rs:503,553` |

**Implementation order**: OPT-2 → DHAT-4 → OPT-4 → DHAT-5

## Detailed Optimization Breakdown

### OPT-1: Cache Land Types in CardCache (CPU: 4.5%, Allocation: minimal)

**Problem**: `ManaEngine::update()` repeatedly compares `card.name` against basic land names:

```rust
// Current: String comparison repeated for every card, every update
if card.name == "Plains" { /* ... */ }
if card.name == "Island" { /* ... */ }
// ... (5 comparisons per card per update)
```

**Solution**: Add flags to CardCache (computed once at card load):

```rust
// In CardCache (src/core/card.rs)
pub struct CardCache {
    // ... existing fields ...

    // Basic land type flags
    pub name_is_plains: bool,
    pub name_is_island: bool,
    pub name_is_swamp: bool,
    pub name_is_mountain: bool,
    pub name_is_forest: bool,
}

// In CardCache::new()
name_is_plains: card.name.eq_ignore_ascii_case("Plains"),
name_is_island: card.name.eq_ignore_ascii_case("Island"),
// ...
```

**Usage in ManaEngine::update()**:

```rust
// After: O(1) flag check (already in cache line)
if card.cache.name_is_plains { /* ... */ }
if card.cache.name_is_island { /* ... */ }
```

**Impact**:
- CPU: 4.5% reduction (5.7B → 5.4B instructions in ManaEngine::update)
- Allocations: None (flags are booleans, no heap allocation)
- Cache locality: Flags likely fit in same cache line as existing CardCache

**Estimated time**: 1-2 hours
- Add flags to CardCache struct (15 min)
- Update CardCache::new() to compute flags (15 min)
- Update ManaEngine::update() to use flags (30 min)
- Test and validate (30 min)

---

### OPT-7: Reuse Candidates Vec (CPU: 4.3%, Allocation: ~2 MB)

**Problem**: `GreedyManaResolver::check_payment()` allocates a fresh Vec for candidates **5 times per spell** (once per color):

```rust
// Current: New Vec allocation per color iteration
for color in [White, Blue, Black, Red, Green] {
    let candidates: Vec<SourceId> = self.mana_sources.iter()
        .filter(|s| s.produces_color(color))
        .collect(); // NEW ALLOCATION
    // ...
}
```

**Solution**: Reuse a single Vec across iterations:

```rust
// In GreedyManaResolver struct
struct GreedyManaResolver {
    // ... existing fields ...
    candidates_buffer: Vec<SourceId>, // Reusable buffer
}

// In check_payment()
for color in [White, Blue, Black, Red, Green] {
    self.candidates_buffer.clear(); // Reuse existing allocation
    self.candidates_buffer.extend(
        self.mana_sources.iter()
            .filter(|s| s.produces_color(color))
            .map(|s| s.id)
    );
    // ...
}
```

**Impact**:
- CPU: 4.3% reduction (less allocator overhead, better cache locality)
- Allocations: ~2 MB → ~0.5 MB (one allocation per spell instead of 5)
- Additional benefit: Fewer cache misses (same buffer reused)

**Estimated time**: 30 minutes
- Add buffer field to struct (5 min)
- Update check_payment() to reuse buffer (15 min)
- Test and validate (10 min)

---

### DHAT-1: Pre-size Vec at mana_engine.rs:550 (CPU: 0.5-1.0%, Allocation: 15-20 MB)

**Problem**: Vec starts at capacity 0 and grows incrementally for dual land colors:

```rust
// Current: Vec starts empty, grows 0 → 1 → 2 (two reallocations)
let mut colors = Vec::new();

for subtype in &card.subtypes {
    let color = match subtype.as_str() {
        "Plains" => Some(ManaColor::White),
        "Island" => Some(ManaColor::Blue),
        // ...
    };
    if let Some(c) = color {
        colors.push(c); // Triggers reallocation
    }
}
```

**Solution**: Pre-size Vec based on typical dual land:

```rust
// After: One allocation with correct size
let mut colors = Vec::with_capacity(5); // Most dual lands have 2, allow for expansion

for subtype in &card.subtypes {
    // ... same logic ...
    if let Some(c) = color {
        colors.push(c); // No reallocation needed
    }
}
```

**Impact**:
- CPU: 0.5-1.0% reduction (fewer reallocations, less allocator overhead)
- Allocations: 27 MB → ~7-12 MB (one allocation instead of 2-3 per Vec)
- This is the **#1 allocation site** (31% of total allocations)

**Estimated time**: 5 minutes (literally a one-line change!)

---

### OPT-8: HashSet for tap_order Lookup (CPU: 5.0%, Allocation: minimal)

**Problem**: `GreedyManaResolver::check_payment()` uses `Vec::contains()` for tap order lookups, resulting in O(n²) behavior:

```rust
// Current: O(n²) - linear search for each candidate
let tap_order = self.compute_tap_order();
for candidate in candidates {
    if tap_order.contains(&candidate.id) {
        score += 10; // Penalty for tapping preferred sources
    }
}
```

**Solution**: Convert tap_order to HashSet for O(1) lookups:

```rust
// After: O(n) - constant-time lookup
let tap_order = self.compute_tap_order();
let tap_order_set: HashSet<_> = tap_order.iter().copied().collect();

for candidate in candidates {
    if tap_order_set.contains(&candidate.id) {
        score += 10;
    }
}
```

**Impact**:
- CPU: 5.0% reduction (O(n²) → O(n) - most significant single optimization!)
- Allocations: One HashSet per spell (~100 bytes vs thousands of comparisons)
- **This is the highest CPU impact optimization**

**Estimated time**: 1 hour
- Convert tap_order to HashSet (15 min)
- Update all lookup sites (30 min)
- Test and validate (15 min)

---

### DHAT-2: Pre-size Vecs in game_loop (CPU: 0.5-1.0%, Allocation: 5-10 MB)

**Problem**: Various Vecs in game_loop.rs grow incrementally:

```rust
// Current: Multiple Vecs start empty and grow
let mut actions = Vec::new(); // Line 1413
let mut abilities = Vec::new(); // Line 3111
```

**Solution**: Pre-size based on typical game phase:

```rust
// After: Pre-size based on game phase
let mut actions = Vec::with_capacity(20); // Typical main phase
let mut abilities = Vec::with_capacity(5); // Typical permanent
```

**Impact**:
- CPU: 0.5-1.0% reduction (fewer reallocations)
- Allocations: 5-10 MB reduction (8-10 MB → 2-5 MB)

**Estimated time**: 15 minutes
- Identify all Vec::new() in hot paths (5 min)
- Add with_capacity() with appropriate sizes (5 min)
- Test and validate (5 min)

---

### DHAT-3: SmallVec for Dual Land Colors (CPU: 0.5-1.0%, Allocation: 8-12 MB)

**Problem**: Most dual lands have exactly 2 colors, but we heap-allocate Vec for them:

```rust
// Current: Heap allocation even for 2-color Vec
let mut colors = Vec::with_capacity(5);
```

**Solution**: Use SmallVec to keep small collections on stack:

```rust
use smallvec::{SmallVec, smallvec};

// After: Stack allocation for ≤5 colors (all dual lands)
let mut colors = SmallVec::<[ManaColor; 5]>::new();
```

**Impact**:
- CPU: 0.5-1.0% reduction (stack allocation is faster)
- Allocations: 8-12 MB reduction (most dual lands never touch heap)
- Better cache locality (stack-allocated data)

**Estimated time**: 30 minutes
- Add smallvec dependency (5 min)
- Replace Vec with SmallVec at key sites (15 min)
- Test and validate (10 min)

**Other SmallVec candidates**:
- Blocker lists: `SmallVec<[CardId; 3]>` (most have 0-2 blockers)
- Target lists: `SmallVec<[CardId; 2]>` (most spells have 0-1 targets)
- Mana production: `SmallVec<[Mana; 4]>` (most produce 1-3 mana)

---

## Implementation Strategy

### Immediate Next Steps (Today)

1. **DHAT-1**: Pre-size Vec at `mana_engine.rs:550` (5 min) ← Start here!
2. **OPT-7**: Reuse candidates Vec in mana payment (30 min)
3. Run benchmarks to measure impact

**Expected results after these two**:
- 4-5% CPU reduction
- 17-22 MB allocation reduction
- 35 minutes of work

### This Week

4. **OPT-1**: Cache land types in CardCache (1-2 hours)
5. **OPT-8**: HashSet for tap_order lookup (1 hour) ← Highest single CPU impact!
6. **DHAT-2**: Pre-size Vecs in game_loop (15 min)

**Expected cumulative results**:
- 15-16% CPU reduction
- 22-32 MB allocation reduction
- ~4 hours of work

### Next Week

7. **DHAT-3**: SmallVec for dual land colors (30 min)
8. **OPT-2**: Cache color counts in ManaPool (2-3 hours)
9. **OPT-4**: Pre-validate before unwinding (2-3 hours)

**Expected cumulative results**:
- 18-20% CPU reduction
- 35-45 MB allocation reduction (50% reduction from current baseline!)
- ~9 hours of work

## Validation Plan

For each optimization:

1. **Before**: Run both profilers
   ```bash
   make callgrindprofile > before_cpu.txt
   make dhatprofile > before_alloc.txt
   ```

2. **Implement**: Make the change

3. **After**: Run both profilers again
   ```bash
   make callgrindprofile > after_cpu.txt
   make dhatprofile > after_alloc.txt
   ```

4. **Validate**:
   - Compare instruction counts from Callgrind
   - Compare allocation totals from DHAT
   - Run `make validate` to ensure correctness
   - Check win rate determinism (should stay ~63.1%)

5. **Document**: Update tracking issue with results

## Conclusion

The combination of CPU and allocation profiling reveals a clear picture:

### What's Already Done ✅
- **String allocation cache (CardCache)**: Eliminated 94.2% of allocations
- **Performance**: 3.5x speedup (31 → 109 games/sec)

### What's Next 🎯
- **Vec operations**: Target the remaining 86.4 MB
- **CPU hotspots**: Focus on ManaEngine, mana payment, and spell casting
- **Quick wins**: 5 optimizations in ~4 hours → 15% CPU reduction + 22-32 MB reduction

### Key Insight 💡
**The CardCache optimization proved that caching works**. Now we apply the same principle to:
- Land type flags (OPT-1) ← same pattern as existing cache
- Vec reuse (OPT-7) ← cache at spell level instead of card level
- Pre-sizing (DHAT-1, DHAT-2) ← allocate once with correct size

**Expected final state** (after Phase 1 + Phase 2):
- CPU: ~85% of current (15-20% reduction)
- Allocations: ~40-50 MB (50% reduction from 86 MB)
- Throughput: ~130-140 games/sec (another 1.2-1.3x speedup)

---

**Recommendation**: Start with DHAT-1 (5 min fix, 15-20 MB impact), then OPT-7 (30 min, 4.3% CPU + 2 MB), then proceed through Phase 1 in order.
