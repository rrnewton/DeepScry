---
title: Reduce allocations in ManaEngine::update() hot path
status: closed
priority: 3
issue_type: task
labels:
- optimization
- performance
created_at: 2025-11-02T22:35:04.738643662+00:00
updated_at: 2025-11-02T22:55:39.806336837+00:00
closed_at: 2025-11-02T22:55:39.806336697+00:00
---

# Description

## Allocation Hotspot: ManaEngine::update() - RESOLVED

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

## Solution Implemented (Commit d071ccf)

### Changes Made

1. **Removed player_id from ManaEngine struct**
   - Changed constructor: `new()` instead of `new(player_id)`
   - Made `update()` take player_id as parameter
   - Allows single engine to be reused for different players

2. **Stored reusable ManaEngine in GameLoop**
   - Added `mana_engine: ManaEngine` field to `GameLoop` struct
   - Initialized once in constructor
   - Reused across all mana availability checks

3. **Pre-allocated vector capacity**
   ```rust
   self.simple_sources.reserve(10);      // Typical: 5-10 lands
   self.complex_sources.reserve(5);      // Typical: 0-5 mana dorks/rocks
   self.mana_sources.reserve(15);        // Combined capacity
   ```

### Call Sites Updated
- `get_castable_spells()`: Now uses `self.mana_engine`
- `get_activatable_abilities()`: Now uses `self.mana_engine`
- `get_available_spell_abilities()`: Changed to `&mut self`

## Measured Impact (2025-11-02_#593)

**Before (1f600d0) vs After (d071ccf):**

### Simple Deck (simple_bolt.dck)
- **Bytes/game**: 305,918 → 243,674 (**-20.3%**)
- **Bytes/turn**: 43,702 → 34,810 (**-20.3%**)

### Old School Decks (Complex Games)

**Mono Black vs The Deck** (32 turns/game):
- **Bytes/game**: 1,247,852 → 811,128 (**-35.0%**)
- **Bytes/turn**: 38,995 → 25,347 (**-35.0%**)

**White Weenie Mirror** (56 turns/game):
- **Bytes/game**: 2,041,820 → 1,249,704 (**-38.8%**)
- **Bytes/turn**: 36,461 → 22,316 (**-38.8%**)

**Jeskai Aggro vs Troll Disk** (39 turns/game):
- **Bytes/game**: 1,784,466 → 1,264,846 (**-29.1%**)
- **Bytes/turn**: 45,755 → 32,431 (**-29.1%**)

### Performance Improvements
- **Speed**: Old school benchmarks showed **15-16% performance improvement**
- **Throughput**: Jeskai benchmark went from 939 → 1,117 games/sec

## Key Insights

1. **Longer games benefit more**: 38.8% reduction for 56-turn games vs 20.3% for 7-turn games
2. **Capacity retention works**: Single engine retains Vec capacity across multiple updates
3. **Per-turn overhead reduced significantly**: 29-39% reduction in bytes/turn for complex games

## Status

✅ **RESOLVED** - Commit d071ccf implements the optimization with excellent results.

**Test Results**: All 404 tests passed.

## References

- Heaptrack profile: `/tmp/heaptrack_analysis.txt`
- Implementation: commit d071ccf
- Benchmark results: `experiment_results/perf_history.csv`
- Tracking: mtg-2 (optimization tracking)
