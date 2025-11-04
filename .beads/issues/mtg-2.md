---
title: Optimization and performance tracking
status: open
priority: 1
issue_type: epic
labels:
- tracking
created_at: 2025-10-26T21:06:34+00:00
updated_at: 2025-11-04T20:10:00.000000000+00:00
---

# Description

Track performance optimization work for MTG Forge Rust.

**Current performance as of 2025-11-04_#713(1961e96):**

*Simple deck (simple_bolt.dck):*
- **Fresh Mode**: 5,520 games/sec, avg 7 turns/game, 232KB/game, 33.1KB/turn
- **Snapshot Mode**: 19,676 games/sec (3.6x faster via clone)
- **Rewind Mode**: 298,854 games/sec (54.1x faster via undo, +52% vs previous!)
- **Rewind + Play Again** (isolates forward gameplay): Details pending

*Old School decks (realistic 32-41 turn games):*
- **Mono Black vs The Deck**: 1,479 games/sec, 32 turns/game, 822KB/game, 25.7KB/turn
- **White Weenie Mirror**: 1,068 games/sec, 41 turns/game, 1.22MB/game, 29.7KB/turn
- **Jeskai Aggro vs Troll Disk**: 1,128 games/sec, 39 turns/game, 1.22MB/game, 31.3KB/turn

**Latest DHAT heap profiling (2025-11-04_#713, 100 iterations rewind+replay):**

Total allocations: 1.10 MB in 27,968 blocks (-2.6% bytes from previous, +6.2% blocks)
Top hotspots:
1. GameLoop::get_available_spell_abilities - ~51KB (4.6%) - helper function allocations
2. Allocator overhead entries (~7-8% each, expected)

**Major wins achieved:**
- ✅ ManaEngine dynamic allocation: 600KB → 0KB (eliminated from top 20!)
- ✅ ManaEngine::update reserve: 70KB → 0KB (eliminated from top 20!)
- ✅ GameLoop abilities buffer: 89KB → 51KB (-43% reduction)
- ✅ RNG serialization: JSON→bincode, 152→56 bytes per turn (63% reduction, 96 bytes/turn saved)
- ✅ RNG SmallVec: Eliminated heap allocation per turn (~40 allocations saved per game)
- ✅ RNG advance_step hotspot: 150KB → eliminated from top hotspots!
- **Total reduction: From 1.86 MB baseline to 1.10 MB (-41% total)**

**Completed optimizations:**
- ✅ mtg-6: Logging allocations (conditional compilation added)
- ✅ mtg-10: Vec reallocations in game loop (SmallVec + fixed arrays)
- ✅ mtg-7: CardDatabase.get_card() returns Arc<CardDefinition>
- ✅ mtg-8: GameStateView already uses borrowing, not cloning
- ✅ mtg-9: CardName and PlayerName use Arc<str>
- ✅ mtg-12: Mana pool calculation optimization (already resolved)
- ✅ mtg-11: Zone transfer operations (investigated, already optimal)
- ✅ mtg-120: ManaEngine allocation hotspot (20-39% reduction, 15-16% faster)
- ✅ mtg-current: ManaResolver Box elimination (3-7% faster)
- ✅ mtg-payment-vecs: Mana payment Vec elimination (85% faster, 1.4M allocations eliminated)
- ✅ mtg-mana-engine-dynamic: ManaEngine dynamic allocation elimination (600KB → 70KB, 3-24% faster)
- ✅ mtg-buffer-reuse: GameLoop + ManaEngine buffer optimization (108KB eliminated, -5% total allocations)
- ✅ mtg-437f88: RNG bincode serialization (96 bytes/turn saved, ~8% of advance_step allocations)
- ✅ mtg-02f1df: RNG SmallVec inline storage (heap allocation eliminated, +52% Rewind mode performance!)

**Low priority (remaining allocations):**
- GameLoop::get_available_spell_abilities helper allocations - 51KB (4.6%)
  - get_lands_in_hand, get_castable_spells return Vecs
  - Would require more API refactoring for modest gains
- Card loading string clones (acceptable one-time cost)
- UndoLog growth (~43KB, necessary for rewind functionality)
- Allocator overhead (expected, unavoidable)

**Future considerations:**
- mtg-13: Arena allocation for per-turn temporaries
- mtg-14: Object pools for reusable objects
- mtg-15: Compile-time feature flags for profiling modes

**Optimization status: Excellent!**
We've achieved a 41% reduction in total allocations (1.86MB → 1.10MB).
Remaining hotspots are all below 5% and require extensive API refactoring for diminishing returns.

The RNG SmallVec optimization had an unexpected but massive impact on Rewind mode performance (+52%),
likely due to improved cache locality and reduced allocator pressure.

See OPTIMIZATION.md for detailed patterns and profiling methodology.

---
**Updated 2025-11-04_#713(1961e96)** - Benchmark and DHAT results after RNG SmallVec optimization
- DHAT: 1.13MB → 1.10MB (-2.6% bytes), 26,328 → 27,968 blocks (+6.2%)
- Rewind mode: 195,963 → 298,854 games/sec (+52.5% improvement!)
- RNG advance_step hotspot eliminated from top allocators
- Blocks increased because SmallVec initialization counts as "allocation" even though it's inline
- **Cumulative achievement: ~760 KB eliminated from baseline (41% reduction)**
