# CPU Hotspot Analysis from Callgrind Profiling

**Date**: 2025-11-08
**Workload**: 250 games, robots mirror deck, sequential mode
**Tool**: Valgrind Callgrind
**Total Instructions**: 18.7 billion

## Executive Summary

The top 3 hotspots account for **68%** of all CPU instructions:

1. **ManaEngine::update**: 5.7B instructions (30.5%) - Scanning battlefield for mana sources
2. **cast_spell_8_step**: 4.4B instructions (23.3%) - Spell casting process
3. **check_payment (GreedyManaResolver)**: 2.7B instructions (14.3%) - Mana payment validation

## Hotspot #1: ManaEngine::update (30.5% of CPU)

### What It Does

**File**: `mtg-engine/src/game/mana_engine.rs:254-353`

**Purpose**: Scans the battlefield to identify and categorize mana-producing permanents.

**Algorithm**:
```
For each card on battlefield:
    1. Check if it's owned by current player
    2. Determine if it produces mana (land or creature with mana ability)
    3. Check for summoning sickness (creatures only)
    4. Classify as simple (basic land) or complex (dual/any-color)
    5. Parse mana production from:
       - Card name (Plains → W, Mountain → R)
       - CardCache.mana_production (precomputed from oracle text)
       - Subtypes (dual lands: Taiga has Plains+Mountain subtypes)
    6. Store in categorized lists
```

**Frequency**: Called every time game state changes (every spell cast, every combat)

### Current Performance Issues

**Problem 1: Repeated String Comparisons** (lines 282, 455-465)
```rust
if let Some(color_char) = get_simple_mana_color(card.name.as_str()) {
    // get_simple_mana_color does string comparison:
    match land_name {
        "Plains" => Some('W'),
        "Island" => Some('U'),
        // ... more comparisons
    }
}
```
- Called for every battlefield card
- String comparisons are O(n) where n = string length
- Repeated for same cards across many game states

**Problem 2: Subtype Iteration for Dual Lands** (lines 533-545)
```rust
for subtype in &card.subtypes {
    let color = match subtype.as_str() {
        "Plains" => Some(ManaColor::White),
        "Island" => Some(ManaColor::Blue),
        // ... more comparisons
    }
}
```
- Iterates Vec<String> for every dual land
- Performs string comparisons inside loop
- Allocates ManaColors bitfield for each dual land

**Problem 3: Vec Growth** (lines 258-261)
```rust
self.simple_sources.clear();
self.complex_sources.clear();
self.mana_sources.clear();
```
- Three Vec allocations that grow during update
- Even with retained capacity, push() still does bounds checking
- ManaSource structs are 32+ bytes each

### Optimization Opportunities

**OPT-1: Cache Land Type in CardCache** (EASY - ~15% reduction)

Currently:
```rust
// In ManaEngine::update
if let Some(color_char) = get_simple_mana_color(card.name.as_str()) {
    // String comparison every time
}
```

Proposed:
```rust
// In CardCache (computed once at card load)
pub struct CardCache {
    // ... existing fields
    pub basic_land_color: Option<ManaColor>,  // None if not basic land
    pub dual_land_colors: ManaColors,          // Empty if not dual land
}

// In ManaEngine::update (just read cached field)
if let Some(color) = card.cache.basic_land_color {
    // Zero-cost lookup, no string comparison
}
```

**Impact**: Eliminates ~95% of string comparisons in this function
- Current: 30.5% of total CPU
- After: ~26% of total CPU (4-5% reduction)

---

**OPT-2: Pre-size Vecs with with_capacity()** (TRIVIAL - ~2% reduction)

Currently:
```rust
self.simple_sources.clear();  // Capacity retained but starts at len=0
```

Proposed:
```rust
// In ManaEngine::new()
Self {
    simple_sources: Vec::with_capacity(20),    // Typical ~10-15 lands
    complex_sources: Vec::with_capacity(5),     // Typical ~0-3 dual lands
    mana_sources: Vec::with_capacity(20),       // Same as simple+complex
    // ...
}
```

**Impact**: Reduces Vec reallocation overhead
- Saves ~1-2% of instructions in this function
- 0.3-0.6% of total CPU

---

**OPT-3: SmallVec for Dual Land Color Collection** (MEDIUM - ~1% reduction)

Currently:
```rust
let mut colors = ManaColors::new();  // Heap-allocated bitfield wrapper
for subtype in &card.subtypes { /* ... */ }
```

Proposed - cache in CardCache instead (same as OPT-1):
```rust
// Read from cache instead of recomputing
if !card.cache.dual_land_colors.is_empty() {
    // Already computed, just use it
}
```

**Impact**: Eliminates Vec iteration + bitfield allocation
- Saves ~1% of instructions in this function
- 0.3% of total CPU

---

## Hotspot #2: cast_spell_8_step (23.3% of CPU)

### What It Does

**File**: `mtg-engine/src/game/actions.rs:640-771`

**Purpose**: Implements MTG's 8-step spell casting process.

**Algorithm**:
```
1. Verify card is in hand
2. Move card to stack
3. Choose targets (callback to controller)
4. (Skipped: Divide effects)
5. Determine total cost
6. Compute tap order using GreedyManaResolver
7. Tap mana sources and add to pool
8. Pay cost from pool
   If any step fails → Unwind (move card back, untap sources, clear pool)
```

**Frequency**: Once per spell cast (~3-5 times per turn, ~100-150 per game)

### Current Performance Issues

**Problem 1: Error Recovery Overhead** (lines 687-764)
```rust
if !resolver.compute_tap_order(&mana_cost, mana_sources, &mut sources_to_tap) {
    // Unwind: move card back to hand
    self.move_card(card_id, Zone::Stack, Zone::Hand, player_id)?;
    // ... error handling
}

for &source_id in &sources_to_tap {
    if let Err(e) = self.tap_for_mana_for_cost(player_id, source_id, &mana_cost) {
        // Unwind: move card back, untap all tapped sources, clear pool
        // ... complex error recovery (lines 700-726)
    }
}

if let Err(e) = player.mana_pool.pay_cost(&mana_cost) {
    // Unwind again: move card, untap sources, clear pool
    // ... duplicate error recovery (lines 734-763)
}
```

**Issues**:
- Three separate error recovery paths with duplicated logic
- Each recovery involves zone moves, undo logging, Vec operations
- Error recovery code is ~80 lines vs ~20 lines of happy path

**Problem 2: Repeated ManaSource Iteration**
- `compute_tap_order()` iterates all sources
- `tap_for_mana_for_cost()` called for each source in tap order
- Each tap logs to undo_log (string formatting for debug)

**Problem 3: Vec Allocations**
```rust
let mut sources_to_tap = Vec::new();  // Allocates every spell cast
let mut tapped_sources = Vec::new();  // Allocates for error tracking
```

### Optimization Opportunities

**OPT-4: Pre-validate Before Unwinding** (MEDIUM - ~5% reduction)

Currently: Optimistic execution with expensive unwind paths

Proposed: Validate upfront, execute only if certain
```rust
// Quick validation (no allocations)
if !mana_engine.can_pay(&mana_cost) {
    return Err(MtgError::InsufficientMana);  // Early return, no unwind
}

// Now we know it's possible - do the actual tapping
// (Error recovery still possible for edge cases, but rare)
```

**Impact**: Avoids 90% of unwind operations
- Saves ~5% of instructions in this function
- 1.2% of total CPU

---

**OPT-5: Reuse Tap Order Vec** (TRIVIAL - ~0.5% reduction)

Currently: Allocates Vec for every spell cast

Proposed: Add to GameState or Controller
```rust
pub struct GameState {
    // ... existing fields
    tap_order_buffer: Vec<CardId>,  // Reused across spell casts
}

// In cast_spell_8_step:
self.tap_order_buffer.clear();
resolver.compute_tap_order(&mana_cost, mana_sources, &mut self.tap_order_buffer)
```

**Impact**: Eliminates Vec allocation on spell cast
- Saves ~0.5% of instructions in this function
- 0.1% of total CPU

---

**OPT-6: Batch Tap Operations** (MEDIUM - ~2% reduction)

Currently: Individual tap calls with undo logging for each

Proposed: Batch tap, single undo log entry
```rust
// Tap all at once
fn tap_sources_for_spell(&mut self, sources: &[CardId]) -> Result<()> {
    let prior_log_size = self.logger.log_count();

    for &source_id in sources {
        self.cards.get_mut(source_id)?.tap();
        // Mana added to pool
    }

    // Single undo log entry for all taps
    self.undo_log.log(GameAction::TapMultipleSources {
        sources: sources.to_vec(),  // Allocates once for undo
    }, prior_log_size);

    Ok(())
}
```

**Impact**: Reduces undo_log operations
- Saves ~2% of instructions in this function
- 0.5% of total CPU

---

## Hotspot #3: GreedyManaResolver::check_payment (14.3% of CPU)

### What It Does

**File**: `mtg-engine/src/game/mana_payment.rs:246-390`

**Purpose**: Determines if a mana cost can be paid and computes tap order.

**Algorithm**:
```
1. bounds_check_payment() - Fast rejection tests
   - Total mana available vs needed
   - Color availability checks

2. try_greedy_payment() - Greedy algorithm
   For each color requirement (W, U, B, R, G):
       - Find all sources that can produce that color
       - Score sources (Fixed < Choice < AnyColor)
       - Sort by score
       - Tap best sources first

   For colorless requirement:
       - Tap Wastes

   For generic requirement:
       - Tap any remaining sources
```

**Frequency**: Called for every spell the AI considers (~20-50 times per turn)

### Current Performance Issues

**Problem 1: Vec Allocation for Candidates** (lines 296-306)
```rust
let mut tap_for_color = |color: ManaColor, amount: u8| {
    let mut candidates: Vec<(usize, u8)> = sources
        .iter()
        .enumerate()
        .filter(|(_, s)| /* ... */)
        .map(|(idx, s)| (idx, Self::score_for_color(&s.production, color)))
        .collect();  // ALLOCATES EVERY TIME

    candidates.sort_by_key(|(_, score)| *score);  // SORTS EVERY TIME
    // ...
};
```

**Impact**:
- Allocates Vec for each color (5 colors = 5 allocations per spell)
- Typical land count: 10-15 sources
- Each allocation: 10-15 * 16 bytes = 160-240 bytes
- Total per spell: ~800-1200 bytes allocated
- With ~100-150 spells considered per game: ~100-180 KB allocated

**Problem 2: Repeated scoring_for_color() calls**
- Scores are recomputed for every color
- Same source scored multiple times
- Scoring involves pattern matching on ManaProductionKind

**Problem 3: Linear Search for card_id in tap_order**
```rust
&& !tap_order.contains(&s.card_id)  // O(n) search, called for each source
```
- `Vec::contains()` is O(n) where n = tap_order length
- Called inside filter for every source candidate
- Typical tap_order size: 3-5 cards
- Total O(n²) behavior: sources × tap_order_length

### Optimization Opportunities

**OPT-7: Pre-allocate and Reuse Candidates Vec** (EASY - ~30% reduction in this function)

Currently: Allocates Vec for each color

Proposed: Reuse single Vec
```rust
impl GreedyManaResolver {
    // Add buffer field
    candidates_buffer: RefCell<Vec<(usize, u8)>>,
}

fn try_greedy_payment(&self, ...) -> bool {
    let mut candidates = self.candidates_buffer.borrow_mut();

    let mut tap_for_color = |color: ManaColor, amount: u8| {
        candidates.clear();  // Reuse existing allocation

        for (idx, s) in sources.iter().enumerate() {
            if /* filters */ {
                candidates.push((idx, Self::score_for_color(&s.production, color)));
            }
        }

        candidates.sort_unstable_by_key(|(_, score)| *score);
        // ...
    };
}
```

**Impact**: Eliminates 5 Vec allocations per spell
- Saves ~30% of instructions in this function
- 4.3% of total CPU

---

**OPT-8: Use HashSet for tap_order lookups** (TRIVIAL - ~10% reduction in this function)

Currently: `Vec::contains()` is O(n)

Proposed: Use small inline bitset or SmallVec with fast contains
```rust
// Option 1: Use rustc_hash::FxHashSet (fast for small sets)
use rustc_hash::FxHashSet;
let mut tapped_set = FxHashSet::default();

// Option 2: Use SmallVec with capacity for stack allocation
use smallvec::SmallVec;
let mut tap_order: SmallVec<[CardId; 8]> = SmallVec::new();
```

**Impact**: Reduces O(n²) to O(n) for tap order checks
- Saves ~10% of instructions in this function
- 1.4% of total CPU

---

**OPT-9: Cache Source Scores** (MEDIUM - ~15% reduction in this function)

Currently: Recomputes score for each color

Proposed: Compute scores once at ManaEngine::update()
```rust
pub struct ManaSource {
    pub card_id: CardId,
    pub production: ManaProduction,
    pub is_tapped: bool,
    pub has_summoning_sickness: bool,
    // New: pre-computed scores for each color
    pub scores: [u8; 5],  // W, U, B, R, G order
}

// In ManaEngine::update, compute scores once:
self.mana_sources.push(ManaSource {
    // ...
    scores: [
        GreedyManaResolver::score_for_color(&production, ManaColor::White),
        GreedyManaResolver::score_for_color(&production, ManaColor::Blue),
        // ... etc
    ],
});
```

**Impact**: Eliminates repeated score computation
- Saves ~15% of instructions in this function
- 2.1% of total CPU

---

## Combined Optimization Impact

### Conservative Estimates

| Optimization | Hotspot Impact | Total CPU Impact | Complexity |
|--------------|----------------|------------------|------------|
| **OPT-1: Cache land types** | -15% ManaEngine | -4.5% total | EASY |
| **OPT-2: Pre-size Vecs** | -2% ManaEngine | -0.6% total | TRIVIAL |
| **OPT-3: Cache dual colors** | -1% ManaEngine | -0.3% total | (covered by OPT-1) |
| **OPT-4: Pre-validate** | -5% cast_spell | -1.2% total | MEDIUM |
| **OPT-5: Reuse tap Vec** | -0.5% cast_spell | -0.1% total | TRIVIAL |
| **OPT-6: Batch tap ops** | -2% cast_spell | -0.5% total | MEDIUM |
| **OPT-7: Reuse candidates Vec** | -30% check_payment | -4.3% total | EASY |
| **OPT-8: HashSet for tapped** | -10% check_payment | -1.4% total | TRIVIAL |
| **OPT-9: Cache scores** | -15% check_payment | -2.1% total | MEDIUM |

**Total estimated speedup: ~15% reduction in CPU instructions**

### Priority Ranking

**High Priority** (Easy + High Impact):
1. **OPT-1**: Cache land types in CardCache - **4.5% total CPU** (1-2 hours)
2. **OPT-7**: Reuse candidates Vec - **4.3% total CPU** (30 minutes)

**Medium Priority** (Trivial + Low Risk):
3. **OPT-8**: HashSet for tap order - **1.4% total CPU** (15 minutes)
4. **OPT-2**: Pre-size Vecs - **0.6% total CPU** (10 minutes)
5. **OPT-5**: Reuse tap order Vec - **0.1% total CPU** (10 minutes)

**Lower Priority** (Medium Complexity):
6. **OPT-9**: Cache source scores - **2.1% total CPU** (1 hour)
7. **OPT-4**: Pre-validate before unwind - **1.2% total CPU** (2 hours)
8. **OPT-6**: Batch tap operations - **0.5% total CPU** (3 hours)

## Implementation Order

### Phase 1: Quick Wins (Total: ~11% reduction, 2-3 hours)
1. OPT-7: Reuse candidates Vec in GreedyManaResolver
2. OPT-1: Add land type caching to CardCache
3. OPT-8: Use HashSet for tap_order lookups
4. OPT-2: Pre-size Vecs in ManaEngine::new()

### Phase 2: Medium Effort (Total: ~4% more, 3-4 hours)
5. OPT-9: Cache source scores in ManaSource
6. OPT-4: Add pre-validation to cast_spell_8_step
7. OPT-5: Reuse tap_order buffer in GameState

### Phase 3: Lower ROI (Total: ~0.5% more, 3+ hours)
8. OPT-6: Batch tap operations (complex, error-prone)

## Next Steps

**Recommended**: Start with Phase 1 optimizations
- Low risk (mostly caching + pre-allocation)
- High reward (~11% CPU reduction)
- Can validate each change independently with `make callgrindprofile`

**Validation Process**:
1. Run `make callgrindprofile` before changes
2. Implement one optimization
3. Run `make callgrindprofile` after
4. Compare instruction counts for the specific function
5. Run `make validate` to ensure correctness

**Success Criteria**:
- ManaEngine::update: Target <4.5B instructions (down from 5.7B) = -21%
- check_payment: Target <1.6B instructions (down from 2.7B) = -40%
- Total: Target <16B instructions (down from 18.7B) = -14.4%
