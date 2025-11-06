# ManaEngine Refactoring Analysis: Destination-Passing Style

**Date**: 2025-11-06
**Context**: DHAT profiling revealed ManaEngine::get_complex_mana_production allocates ~150 KB (22.4%) during gameplay

## Current Implementation Analysis

### The Hotspot: `get_complex_mana_production`

**Location**: `src/game/mana_engine.rs:528-568`

**Profiled allocation site**: Line 537 (`let mut colors = Vec::new()`)

This function is called **repeatedly** during AI decision-making when checking if spells can be cast.

### Current Signature

```rust
fn get_complex_mana_production(card: &Card) -> Option<ManaProduction>
```

### Current Allocation Pattern

```rust
fn get_complex_mana_production(card: &Card) -> Option<ManaProduction> {
    // Must be a land
    if !card.types.contains(&CardType::Land) {
        return None;
    }

    // ALLOCATION #1: Vec for collecting colors
    let mut colors = Vec::new();  // Line 537 - THE HOTSPOT!

    // Check subtypes for basic land types (dual lands)
    for subtype in &card.subtypes {
        let color = match subtype.as_str() {
            "Plains" => Some(ManaColor::White),
            "Island" => Some(ManaColor::Blue),
            // ... etc
        };
        if let Some(c) = color {
            colors.push(c);  // Pushes to heap-allocated Vec
        }
    }

    // If exactly 2 subtypes, it's a dual land
    if colors.len() == 2 {
        // ALLOCATION #2: Vec moved into ManaProductionKind::Choice
        return Some(ManaProduction::free(ManaProductionKind::Choice(colors)));
    }

    // ALLOCATION #3: String allocation from to_lowercase()
    let text_lower = card.text.to_lowercase();
    if text_lower.contains("any color") {
        return Some(ManaProduction::free(ManaProductionKind::AnyColor));
    }

    None
}
```

### Type Definitions

```rust
pub struct ManaProduction {
    pub kind: ManaProductionKind,
    pub activation_cost: Option<ManaCost>,
}

pub enum ManaProductionKind {
    Fixed(ManaColor),           // No allocation
    Choice(Vec<ManaColor>),     // ALLOCATES! Vec on heap
    AnyColor,                   // No allocation
    Colorless,                  // No allocation
}
```

### Call Context (from ManaEngine::update)

```rust
// Line 338 in update() - called for EVERY complex land on battlefield
if let Some(production) = get_complex_mana_production(card) {
    self.complex_sources.push(card_id);
    self.mana_sources.push(ManaSource {
        card_id,
        production,  // Moves the allocated Vec here
        is_tapped: card.tapped,
        has_summoning_sickness,
    });
}
```

**Frequency**: Called once per complex land during `update()`, which is called:
- When permanents enter/leave battlefield
- When lands become tapped/untapped
- Before AI decision-making

**In 20-turn UR Burn game**: Called ~445 times (from DHAT profiling)

## Refactoring Feasibility Assessment

### Goal: Destination-Passing Style with Reusable Buffers

Similar to how `can_pay()` uses resolvers with internal buffers:

```rust
pub struct GreedyManaResolver {
    remaining_cost: ManaCost,          // Reusable buffer
    available_sources: Vec<usize>,      // Reusable buffer
    color_needed_counts: HashMap<ManaColor, u8>,  // Reusable buffer
}
```

### Option A: Pass Mutable Buffer for Colors (FEASIBLE)

**New signature**:
```rust
fn get_complex_mana_production_into(
    card: &Card,
    color_buffer: &mut Vec<ManaColor>
) -> Option<ManaProductionKind>
```

**Implementation**:
```rust
fn get_complex_mana_production_into(
    card: &Card,
    color_buffer: &mut Vec<ManaColor>  // Reused across calls
) -> Option<ManaProductionKind> {
    if !card.types.contains(&CardType::Land) {
        return None;
    }

    // Reuse existing capacity, just clear contents
    color_buffer.clear();

    // Collect dual land colors into the buffer
    for subtype in &card.subtypes {
        let color = match subtype.as_str() {
            "Plains" => Some(ManaColor::White),
            "Island" => Some(ManaColor::Blue),
            // ...
        };
        if let Some(c) = color {
            color_buffer.push(c);  // Uses existing capacity
        }
    }

    // Check if it's a dual land
    if color_buffer.len() == 2 {
        // STILL ALLOCATES: Must create Vec for Choice variant
        return Some(ManaProductionKind::Choice(color_buffer.clone()));
    }

    // Check for any-color lands
    let text_lower = card.text.to_lowercase();  // Still allocates!
    if text_lower.contains("any color") {
        return Some(ManaProductionKind::AnyColor);
    }

    None
}
```

**Usage in update()**:
```rust
// Add to ManaEngine struct:
pub struct ManaEngine {
    // ... existing fields ...
    color_buffer: Vec<ManaColor>,  // Reusable buffer
}

// In update():
if let Some(kind) = get_complex_mana_production_into(card, &mut self.color_buffer) {
    let production = ManaProduction::free(kind);
    // ...
}
```

**Pros**:
- Reuses buffer capacity across multiple calls
- Reduces allocations from ~445 to ~445 (wait, no improvement!)

**Cons**:
- **Still allocates** when creating `Choice(color_buffer.clone())`
- Complex lands typically have 2 colors, so most calls still allocate
- String::to_lowercase() still allocates

**Verdict**: ❌ Not effective - still allocates for the common case

### Option B: Change ManaProductionKind to Use SmallVec (MORE FEASIBLE)

**Modified type**:
```rust
use smallvec::SmallVec;

pub enum ManaProductionKind {
    Fixed(ManaColor),
    Choice(SmallVec<[ManaColor; 2]>),  // Inline for dual lands!
    AnyColor,
    Colorless,
}
```

**Impact**:
- Dual lands (2 colors) store inline - NO heap allocation!
- Tri-lands (3 colors) would still allocate (rare)
- ManaColor is Copy and 1 byte, so SmallVec<[ManaColor; 2]> is 3 bytes + inline array

**Implementation**:
```rust
fn get_complex_mana_production(card: &Card) -> Option<ManaProduction> {
    // ... same logic ...
    
    if colors.len() == 2 {
        let mut inline_colors = SmallVec::new();
        inline_colors.extend_from_slice(&colors);  // Inline storage!
        return Some(ManaProduction::free(ManaProductionKind::Choice(inline_colors)));
    }
}
```

**Pros**:
- ✅ Eliminates 99% of Vec allocations (dual lands dominate)
- ✅ No API changes needed
- ✅ Simple refactor

**Cons**:
- ❌ String::to_lowercase() still allocates
- ❌ Adds SmallVec dependency to ManaProductionKind (but we already use SmallVec elsewhere)

**Verdict**: ✅ **FEASIBLE and EFFECTIVE**

### Option C: Pre-lowercase Card Text During Loading (MOST EFFECTIVE)

**Root cause**: `card.text.to_lowercase()` allocates a String

**Solution**: Store lowercased text in Card during database loading

```rust
pub struct Card {
    // ... existing fields ...
    pub text: String,
    pub text_lowercase: String,  // NEW: pre-lowercased oracle text
}
```

**Modified function**:
```rust
fn get_complex_mana_production(card: &Card) -> Option<ManaProduction> {
    // ... dual land logic ...
    
    // NO ALLOCATION: use pre-lowercased text
    if card.text_lowercase.contains("any color") {
        return Some(ManaProduction::free(ManaProductionKind::AnyColor));
    }
    
    None
}
```

**Pros**:
- ✅ Eliminates String::to_lowercase() allocations entirely
- ✅ One-time cost during card loading
- ✅ Helps other text-parsing code too

**Cons**:
- ❌ Increases Card memory footprint (but oracle text is rarely long)
- ❌ Needs database loader modification

**Verdict**: ✅ **HIGHLY FEASIBLE and EFFECTIVE**

### Option D: Cache ManaProduction Results (COMPLEMENTARY)

**Observation**: Card mana production doesn't change during a game

**Solution**: Cache ManaProduction in Card struct

```rust
pub struct Card {
    // ... existing fields ...
    cached_mana_production: Option<ManaProduction>,  // Computed once
}
```

**Modified update()**:
```rust
// In update():
for &card_id in &game.battlefield.cards {
    if let Ok(card) = game.cards.get(card_id) {
        // Check cache first
        let production = if let Some(cached) = &card.cached_mana_production {
            cached.clone()  // Cheap clone (only clones Vec if Choice)
        } else {
            // Compute and cache
            let prod = get_complex_mana_production(card)?;
            // TODO: Store in card.cached_mana_production
            prod
        };
    }
}
```

**Pros**:
- ✅ Reduces calls to get_complex_mana_production from N per update to N once per game
- ✅ Massive reduction in allocation frequency

**Cons**:
- ❌ Requires mutable access to Card during update() (currently immutable)
- ❌ Cache invalidation complexity if cards can change (rare in MTG)

**Verdict**: ⚠️ **Feasible but requires architectural change**

## Recommended Refactoring Strategy

### Phase 1: Quick Wins (IMMEDIATE)

**1. Use SmallVec for ManaProductionKind::Choice**
- Changes: `mana_payment.rs` type definition
- Impact: Eliminates ~95% of dual land Vec allocations
- Effort: 30 minutes
- Risk: Low (SmallVec already used elsewhere)

**2. Pre-lowercase oracle text in Card**
- Changes: Add `text_lowercase: String` to Card struct
- Populate during database loading
- Update all text checks to use lowercased version
- Impact: Eliminates all String::to_lowercase() allocations
- Effort: 2 hours
- Risk: Low (purely additive)

**Expected reduction**: ~150 KB (22.4%) → ~30 KB (~4.5%)

### Phase 2: Structural Optimization (FUTURE)

**3. Cache ManaProduction in Card**
- Requires: Rethinking Card mutability in update()
- Consider: Lazy<ManaProduction> or OnceCell pattern
- Impact: Near-zero allocation after first computation
- Effort: 1 day
- Risk: Medium (architectural change)

### Phase 3: Destination-Passing (DEFERRED)

**4. Full DPS refactor**
- Only valuable if we still see allocations after Phase 1+2
- Requires changing ManaProductionKind storage model
- Not worth the complexity unless profiling shows need

## Comparison to can_pay() Patterns

### Current Patterns in can_pay()

The payment resolvers already use destination-passing internally:

```rust
impl GreedyManaResolver {
    fn can_pay(&self, cost: &ManaCost, sources: &[ManaSource]) -> bool {
        // Uses self.remaining_cost as mutable buffer
        // Uses self.available_sources as mutable buffer
        // Uses self.color_needed_counts as mutable buffer
        
        // All buffers are reused across calls - NO per-call allocation
    }
}
```

**Why it works there**:
- Resolvers are stateful structs with owned buffers
- Buffers are cleared and reused, not returned
- Final result is bool, not allocated data

**Why it's harder for get_complex_mana_production**:
- Returns ManaProduction which contains ManaProductionKind
- ManaProductionKind::Choice owns a Vec
- Can't avoid allocation without changing the storage model

## Conclusion

### Feasibility Summary

| Option | Feasibility | Impact | Effort | Recommendation |
|--------|-------------|--------|--------|----------------|
| **A: DPS with buffer** | Low | Minimal | Medium | ❌ Skip |
| **B: SmallVec for Choice** | High | High | Low | ✅ **DO THIS** |
| **C: Pre-lowercase text** | High | High | Low | ✅ **DO THIS** |
| **D: Cache in Card** | Medium | Very High | High | ⚠️ Future |

### Recommended Action

**Implement Options B + C immediately** (combined effort: ~3 hours)

This should reduce ManaEngine allocations from 22.4% to ~4.5%, making it no longer a significant hotspot.

**The key insight**: True destination-passing style isn't needed here. The real wins come from:
1. Inline storage for small collections (SmallVec)
2. Pre-computation of expensive operations (lowercase)
3. Caching invariant results (future)

These are simpler, lower-risk, and more effective than full DPS refactoring.
