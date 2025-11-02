---
title: Reduce allocations in ManaEngine::update() hot path
status: open
priority: 3
issue_type: task
labels:
- optimization
- performance
created_at: 2025-11-02T22:35:04.738643662+00:00
updated_at: 2025-11-02T22:35:04.738643662+00:00
---

# Description

## Allocation Hotspot: ManaEngine::update()

## Profile Data (2025-11-02_#584)

Heaptrack profiling of 100 games revealed **ManaEngine::update()** as the primary allocation hotspot in the game loop.

### Profile Summary
- **Total allocations**: 2,141 calls (100 games)
- **Peak heap**: 294.80 KB
- **Runtime**: 0.10s
- **Allocation rate**: 22,072 allocs/sec

### Top Allocation Sources

#### 1. ManaEngine::update() - Vec::push (158 calls)
**Location**: `src/game/mana_engine.rs:295` and `:306`

```rust
self.simple_sources.push(card_id);        // Line 295
self.mana_sources.push(ManaSource { ... }); // Line 306
```

**Call frequency**:
- Called from `get_available_spell_abilities()`
- Invoked multiple times per priority round
- Can be **dozens of times per turn** in complex games

**Call chain**:
```
game_loop::priority_round()
  → get_available_spell_abilities()
    → get_castable_spells() / get_activatable_abilities()
      → ManaEngine::update()
        → Vec::push (reallocations)
```

#### 2. Card instantiation (60 calls, 2.06 KB)
**Location**: `loader/card.rs:168`
- One-time setup cost when loading decks
- Not a runtime hotspot

#### 3. Tokio runtime (1,439 calls, 196.56 KB)
- One-time startup cost
- Not per-game overhead

## Problem

The `ManaEngine::update()` method:
1. Calls `clear()` on vectors (lines 246-249), which retains capacity
2. Rebuilds `simple_sources` and `mana_sources` from scratch each call
3. Pushes to vectors without pre-allocation
4. Gets called multiple times per priority round

While `clear()` retains capacity, we still reallocate when growing beyond previous capacity in longer games.

## Proposed Solution

Pre-size vectors based on typical battlefield sizes:

```rust
pub fn update(&mut self, game: &GameState, player_id: PlayerId) {
    self.simple_capacity = ManaCapacity::default();
    self.simple_sources.clear();
    self.simple_sources.reserve(10);  // Typical: 5-10 lands
    self.conditional_sources.clear();
    self.conditional_sources.reserve(5);  // Typical: 0-5 mana dorks/rocks
    self.mana_sources.clear();
    self.mana_sources.reserve(15);  // Combined
    
    // ... rest of update logic
}
```

## Expected Impact

**Current allocation per turn**: 36-46 KB/turn (from benchmarks)

**Potential savings**:
- Reduce Vec reallocations in hot path
- More stable memory usage
- Better cache locality

**Minimal downside**:
- Small increase in minimum memory (~240 bytes reserved per update)
- Negligible compared to per-turn allocation (36-46 KB)

## Benchmarking Plan

1. Add capacity pre-allocation to `ManaEngine::update()`
2. Re-run heaptrack profile
3. Compare allocation counts and memory usage
4. Measure impact on games/sec throughput

## References

- Heaptrack profile: `/tmp/heaptrack_analysis.txt`
- Benchmark results: commit 1f5f2e5
- Related: mtg-2 (optimization tracking)
