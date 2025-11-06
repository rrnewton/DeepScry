# Phase 2 Allocator API - Realistic DHAT Profiling Analysis
**Date**: 2025-11-06 (Commit: allocator-phase2-profiling)
**Deck**: Old School UR Burn (decks/old_school2/ur_burn.dck)

## Summary

Profiling with a realistic deck (mana, spells, activated abilities) reveals **dramatically different** allocation patterns compared to the simplified Lightning Bolt test:

### Comparison: Simple vs Realistic

| Metric | Simple (Bolt-only) | Realistic (UR Burn) | Change |
|--------|-------------------|---------------------|--------|
| **Total allocations** | 208.49 KB | 669.64 KB | +3.2x |
| **Total blocks** | 681 | 5,966 | +8.8x |
| **UndoLog** | 103.59 KB (49.7%) | 103.56 KB (15.5%) | Same KB, much smaller % |
| **CardZone::add** | 992 B (0.5%) | Not in top 20 (<1%) | Minimal |
| **ManaEngine** | N/A (no mana!) | ~150 KB (22.4%) | **NEW HOTSPOT!** |

## Key Findings

### #1: ManaEngine is the Real Bottleneck

**Total ManaEngine allocations: ~150 KB (22.4%)**

The `get_complex_mana_production` function dominates with many repeated entries:
- #4-#5: 47.28 KB each (7.1% each) - 445 blocks each
- #11-#12: 7.25 KB each (1.1% each) - 68 blocks each  
- #15-#20: 6.58 KB each (1.0% each) - 62 blocks each

**Location**: `src/game/mana_engine.rs:561:32` in `ManaEngine::update`

This is called **every time a player wants to cast a spell** to calculate available mana from complex lands like:
- City of Brass (taps for any color)
- Volcanic Island (dual land)
- Mishra's Factory (activated ability)

### #2: UndoLog Still Significant

**Total UndoLog: 103.56 KB (15.5%)**
- #1: 95.62 KB (14.3%) - actions vector
- #10: 7.97 KB (1.2%) - log sizes vector

Same absolute size as simple test, but now smaller percentage due to increased total allocations.

### #3: CardZone Allocations Still Minimal

CardZone::add **does not appear in top 20** allocation sites (<1% of total).

Even with realistic gameplay including:
- Spell casting
- Cards moving between zones
- Activated abilities
- Discard to hand size

Zone movements **are not a significant allocation hotspot** during gameplay.

## Allocation Breakdown

**Total**: 669.64 KB in 5,966 blocks (avg 114.9 bytes/block)

### Top Categories

1. **ManaEngine** (~150 KB, 22.4%) - Calculating complex mana production
2. **UndoLog** (103.56 KB, 15.5%) - Recording game actions
3. **Allocator overhead** (~159 KB, 23.8%) - Internal allocator structures
4. **Card loading** (6.75 KB, 1.0%) - Database async operations
5. **Other** (~250 KB, 37.3%) - Misc allocations

### ManaEngine Detail

The mana engine creates temporary Vec/HashMap structures for each mana calculation:

```rust
// src/game/mana_engine.rs:561:32
fn get_complex_mana_production(...) -> ManaCost {
    let mut mana_abilities = Vec::new();  // Allocation!
    // ... gather all mana sources
    // ... calculate combinations
}
```

This is called **repeatedly** during gameplay whenever the AI considers casting a spell.

## Critical Insight: Clone vs Gameplay Allocations

**This profiling measures GAMEPLAY allocations, not CLONE allocations.**

In MCTS, the allocation pattern is different:

### Gameplay (what we measured):
```rust
// Run one game from start to finish
game.run() // Allocates in ManaEngine, UndoLog, etc.
```

### MCTS Clone Pattern (what we care about):
```rust
// Clone game state 1000x per second
for simulation in simulations {
    let mut cloned = game.clone();  // <-- What allocates here?
    cloned.simulate_random_playout();
}
```

**Question**: Does `game.clone()` trigger:
- CardZone Vec allocations? (Our Phase 2 target)
- UndoLog allocations? (Per-game transient, probably not cloned)
- ManaEngine allocations? (Probably reconstructed, not cloned)

## Implications for Phase 2

### What We Know

1. **ManaEngine is the gameplay hotspot** (22.4%)
   - But is it cloned or reconstructed in MCTS?
   - If reconstructed, not relevant to clone allocations

2. **UndoLog is significant in gameplay** (15.5%)
   - But it's per-game transient (probably not cloned)
   - Not relevant to MCTS clone performance

3. **CardZone is minimal in gameplay** (<1%)
   - But might be significant in cloning (Vec copy)
   - Still unknown!

### What We Still Don't Know

**The critical unknown remains: What allocates during `GameState.clone()`?**

This profiling improved our understanding of gameplay allocations, but MCTS performance depends on **clone allocations**, not gameplay allocations.

## Recommended Next Steps

### Option A: Profile GameState.clone() Directly (RECOMMENDED)

Create a micro-benchmark that isolates clone behavior:

```rust
let game = setup_typical_game_state();

let _profiler = dhat::Profiler::new_heap();

// Clone 1000 times
for _ in 0..1000 {
    let _cloned = game.clone();
}
```

This will show **exactly** what allocates during cloning, which is what matters for MCTS.

### Option B: Complete GameState<A> and Benchmark

1. Finish manual Serialize/Deserialize for GameState<A>
2. Run parallel benchmark with/without custom allocator
3. Measure actual speedup

### Option C: Optimize ManaEngine First

ManaEngine is 22.4% of allocations. Could:
1. Pre-allocate capacity in mana calculation buffers
2. Cache mana production results
3. Use stack-allocated SmallVec for common cases

But: Is ManaEngine even relevant to MCTS clone performance?

## Conclusion

**Realistic deck profiling reveals new insights:**
- ManaEngine dominates (22.4%) - but in gameplay, not cloning
- UndoLog still significant (15.5%) - but per-game transient
- CardZone still minimal (<1%) - in gameplay

**The critical question remains unanswered:**

What allocates during `GameState.clone()` for MCTS?

**Recommendation**: Profile clone behavior specifically with Option A before proceeding with Phase 2.

## Files

- `mtg-engine/examples/heap_profile_game.rs` - Now uses realistic deck loading
- `dhat-heap.json` - Realistic profiling data (669.64 KB)
- `ai_docs/phase2_realistic_dhat_analysis.md` - This analysis
