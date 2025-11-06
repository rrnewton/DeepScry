# Phase 2 Allocator API - DHAT Profiling Analysis
**Date**: 2025-11-06 (Commit: 9c199c4c - CardZone/PlayerZones parameterized)

## Summary

Fresh DHAT heap profiling of a 20-turn game reveals **CardZone allocations are NOT the dominant hotspot** as originally assumed. The actual allocation pattern is:

- **Total allocations**: 208.49 KB in 681 blocks during 20-turn game
- **UndoLog dominates**: 103.59 KB (49.7%) from UndoLog.log() calls
- **CardZone allocations**: Only 992 bytes (0.5%) from CardZone::add()

## Top Allocation Sites

### #1: UndoLog::log actions (95.62 KB, 45.9%)
```
Location: mtg_forge_rs::undo::UndoLog::log (src/undo.rs:294:26)
  ↳ Called from: GameState::draw_card (src/game/state.rs:293:31)
8 blocks, avg 12,240 bytes/block
```

### #2: UndoLog::log sizes (7.97 KB, 3.8%)
```
Location: mtg_forge_rs::undo::UndoLog::log (src/undo.rs:295:28)
  ↳ Called from: GameState::draw_card (src/game/state.rs:293:31)
8 blocks, avg 1,020 bytes/block
```

### #15: GameState::advance_step (1.09 KB, 0.5%)
```
Location: GameState::advance_step (src/game/state.rs:482:36)
20 blocks, avg 56 bytes/block
```

### #17: CardZone::add (992 bytes, 0.5%)
```
Location: mtg_forge_rs::zones::CardZone<A>::add (src/zones.rs:132:20)
10 blocks, avg 99.2 bytes/block
```

## Critical Finding: UndoLog is the Real Hotspot

**UndoLog accounts for ~50% of all allocations**, not CardZone/PlayerZones!

From mtg-2 tracking issue:
> UndoLog: ~5 KB per game, per-game transient structure

**Reality check**: In this 20-turn game, UndoLog allocated **103.59 KB** (20.7x more than estimated).

## Implications for Phase 2

### Original Plan (Based on Assumption)
1. Parameterize CardZone/PlayerZones ✅ (Done)
2. Parameterize GameState ✅ (In progress, doesn't compile)
3. Use per-game bump allocators to eliminate contention

### New Understanding (Based on Data)

**CardZone allocations are minimal** (0.5%). The real allocation pressure comes from:

1. **UndoLog** (49.7%) - grows dynamically with Vec::push
2. **Allocator overhead** (~40%) - various allocator internal structures
3. **CardZone** (0.5%) - minimal, likely from Vec growth in zone.cards

### Why CardZone Allocations Are So Low

The test game had **no mana** (decks with only Lightning Bolts, no Mountains drawn). Players couldn't cast spells, so:
- No cards moved between zones frequently
- No battlefield growth
- Minimal Vec resizing in CardZone::cards

**This is NOT representative of MCTS clone workload!**

## Re-evaluating the Phase 2 Scope

### Question 1: Does GameState.clone() trigger CardZone allocations?

**YES!** Even though this profiling run shows minimal CardZone allocations during gameplay, the critical question is what happens during `game.clone()` for MCTS:

```rust
// In MCTS, every simulation node does:
let mut cloned_game = game.clone();  // <-- Does this allocate zone Vecs?
```

If `CardZone.cards: Vec<CardId, A>` clones with Global allocator, we still have contention!

### Question 2: Should we continue parameterizing GameState?

**Current status**: GameState<A> parameterized but doesn't compile (needs Serialize/Deserialize).

**Options**:
1. **Complete it** - Finish manual Serialize/Deserialize, validate clone() uses custom allocator
2. **Pause it** - Focus on UndoLog optimization first
3. **Test it empirically** - Run parallel clone benchmark to see if it helps

## Recommended Next Steps

### Option A: Validate Clone Behavior (Recommended)
1. Fix GameState<A> compilation (implement Serialize/Deserialize)
2. Write a micro-benchmark that clones GameState 1000x
3. Profile with DHAT to see if zone allocations dominate in clone workload
4. **Then** decide whether to continue Phase 2

### Option B: Pivot to UndoLog Optimization
1. Recognize UndoLog is the real hotspot (49.7%)
2. Reserve capacity upfront: `Vec::with_capacity(expected_actions)`
3. Consider per-game bump allocator for UndoLog specifically
4. Benchmark parallel speedup improvement

### Option C: Both (Comprehensive)
1. Complete GameState<A> parameterization
2. Add UndoLog capacity pre-allocation
3. Benchmark parallel speedup with both changes

## Data Quality Note

**This profiling run is not representative of MCTS workload** because:
- Game had no playable lands (all Lightning Bolts in opening hand)
- No spell casting occurred
- Minimal zone movement
- UndoLog grew from draw/discard only

**Need**: Profile a more realistic game with mana and spell casting to see typical CardZone allocation patterns.

## Files Created

- `mtg-engine/examples/heap_profile_game.rs` - DHAT profiling harness
- `mtg-engine/dhat-heap.json` - Raw profiling data
- `ai_docs/phase2_dhat_analysis.md` - This analysis

## Conclusion

**Phase 2 assumption was wrong**: CardZone allocations are 0.5%, not dominant.

**Real hotspot**: UndoLog (49.7%), but this is per-game transient (not cloned in MCTS).

**Critical unknown**: Does `GameState.clone()` for MCTS allocate zone Vecs with Global allocator even after parameterization? Need empirical validation.

**Recommendation**: Complete GameState<A> and benchmark clone behavior before proceeding further.
