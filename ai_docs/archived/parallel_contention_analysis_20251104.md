# Parallel Contention Analysis

**Date**: 2025-11-04
**Context**: Investigating 47.4% parallel efficiency (8 physical cores + mimalloc)
**Goal**: Identify sources of contention beyond allocation

## Summary of Findings

**Good news**: No obvious shared state contention sources found in parallel benchmark.
**Bad news**: 47.4% efficiency suggests the problem is primarily:
1. **High allocation frequency** (6.5KB/game) - dominant factor
2. **GameState clone cost** - cloning entire game state for each thread
3. **Possible cache effects** from cloning large structures

## Detailed Analysis

### 1. Shared State Analysis

**Checked for contention sources:**
- ✅ No `Arc<RwLock<_>>` accessed during gameplay
- ✅ No `Arc<Mutex<_>>` accessed during gameplay
- ✅ CardDatabase not accessed during parallel gameplay (cards pre-loaded)
- ✅ GameState uses `RefCell<RNG>` but each thread gets independent clone

**Benchmark pattern:**
```rust
// Line 1103-1105: Each thread gets independent GameState clone
let snapshots: Vec<GameState> = (0..num_threads)
    .map(|_| game.clone())
    .collect();

// Threads run independently - no shared state
snapshots.into_par_iter().map(|mut thread_game| {
    // Each thread modifies its own GameState
    run_forward_gameplay_from_snapshot(&mut thread_game, ...)
})
```

**Conclusion**: Benchmark design is sound - no shared state contention.

### 2. GameState Clone Analysis

**What gets cloned** (from `src/game/state.rs:17`):
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameState {
    pub cards: EntityStore<Card>,              // All cards (deck+battlefield+zones)
    pub players: Vec<Player>,                  // Player state
    pub player_zones: Vec<(PlayerId, PlayerZones)>, // Hand, library, graveyard per player
    pub battlefield: CardZone,                 // Shared battlefield
    pub stack: CardZone,                       // The stack
    pub turn: TurnStructure,                   // Turn/phase info
    pub combat: CombatState,                   // Combat state
    pub rng: RefCell<ChaCha12Rng>,            // RNG (clones seed state)
    pub next_id: EntityIdGenerator,            // ID generator
    pub undo_log: Vec<UndoEntry>,             // Undo stack
    pub logger: GameLogger,                    // Logger state
}
```

**Clone cost breakdown:**

For simple_bolt.dck game at mid-point (96 actions):
- **Cards**: ~40 cards * sizeof(Card) ≈ 40 * 200 bytes = 8KB
- **Zones**: Hand (7 cards) + Library (30 cards) + Graveyard (3 cards) = 40 card IDs * 8 bytes = 320 bytes
- **Undo log**: 96 entries * ~100 bytes = 9.6KB
- **String allocations**: Card names, oracle text (cloned via String)
- **Total estimate**: **15-20KB per clone** (8 clones = 120-160KB per benchmark iteration)

**Problem**: Cloning is expensive, especially:
1. Deep copying all Card structs with String fields (`name`, `text`)
2. Cloning entire undo log (not needed for forward simulation!)
3. Allocating new Vecs for all zones

### 3. Cache Effects

**Potential cache issues:**

1. **Clone amplifies cache pressure**:
   - 8 threads each clone 15-20KB of game state
   - Total: 120-160KB cloned data per iteration
   - This pollutes L2/L3 cache, evicting hot data

2. **False sharing unlikely**:
   - Each thread has independent GameState
   - No shared cache lines between threads

3. **TLB pressure**:
   - Many small allocations during clone (Strings, Vecs)
   - Creates TLB misses when accessing scattered memory

### 4. Allocation Breakdown

**Per-game allocation sources** (6.5KB total):

From previous analysis:
1. **GameState clones**: Not measured in 6.5KB (clone happens outside timing)
2. **Gameplay allocations**: 6.5KB during forward simulation
   - Vec allocations for choices, targets, etc.
   - String allocations for logging (even with Silent mode?)
   - Temporary collections in AI/controller code

**Critical insight**: The 6.5KB is ONLY forward gameplay, not including the 15-20KB clone cost!

## Optimization Priorities

### Priority 1: Reduce GameState Clone Cost (Immediate Impact)

**Option A: Selective cloning** (best for MCTS)
- Don't clone undo_log for forward simulations
- Don't clone logger state
- Consider arena-allocated Card pool (share definition, clone only state)

**Option B: Copy-on-write for large structures**
- Use `Rc<[Card]>` for read-only card definitions
- Clone only mutable game state

**Estimated impact**: Reduce clone cost from 15-20KB to 5-8KB (~60% reduction)

### Priority 2: Continue mtg-2 Allocation Reduction

**Target**: <1KB per game (currently 6.5KB)

Focus areas:
1. Eliminate Vec allocations in hot paths (use destination-passing style)
2. Expand SmallVec inline storage
3. Pool frequently allocated objects
4. Remove logging allocations (ensure Silent mode is truly zero-alloc)

**Estimated impact**: With <1KB/game + lighter clones, could reach 70-80% efficiency

### Priority 3: MCTS-Specific Optimizations

**For parallel MCTS**:
1. Use bump allocators per thread (Phase 2 from mtg-a6ca26)
2. Snapshot only minimal game state needed for simulation
3. Consider using GameState references where possible (tricky with lifetimes)

## Action Items

1. **File issue**: Optimize GameState clone for MCTS simulations
   - Remove undo_log from clones
   - Measure clone cost independently
   - Implement selective cloning

2. **Continue mtg-2**: Drive down 6.5KB allocation to <1KB
   - Focus on hot paths identified in previous profiling
   - Prioritize Vec allocations

3. **Measure clone overhead**:
   - Add benchmark that only measures clone cost
   - Separate clone time from gameplay time

4. **Revisit after optimization**:
   - Once clones are lighter + allocations < 1KB, re-measure parallel efficiency
   - Should see significant improvement

## Expected Outcomes

**After GameState clone optimization + mtg-2 completion:**
- Clone cost: 15-20KB → 5-8KB (60% reduction)
- Gameplay allocations: 6.5KB → <1KB (85% reduction)
- **Predicted parallel efficiency: 70-80%** (vs current 47.4%)

**With Phase 2 (bump allocators):**
- Nearly zero allocation during gameplay
- Minimal cache pressure
- **Predicted parallel efficiency: 85-95%**

## References

- mtg-a6ca26: Parallel MCTS optimization tracking issue
- mtg-2: Main optimization tracking (allocation reduction)
- Benchmark: benches/game_benchmark.rs:1103-1105 (GameState cloning)
- GameState: src/game/state.rs:17 (structure definition)
