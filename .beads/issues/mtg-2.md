---
title: Optimization and performance tracking
status: open
priority: 1
issue_type: epic
labels:
- tracking
created_at: 2025-10-26T21:06:34+00:00
updated_at: 2025-11-04T19:30:54.806375283+00:00
---

# Description

Track performance optimization work for MTG Forge Rust.

**Current performance as of 2025-11-04_#709(62bb5fd):**

*Simple deck (simple_bolt.dck):*
- **Fresh Mode**: 5,593 games/sec, avg 7 turns/game, 235KB/game, 33.6KB/turn
- **Snapshot Mode**: 19,430 games/sec (3.5x faster via clone)
- **Rewind Mode**: 195,963 games/sec (35.0x faster via undo)
- **Rewind + Play Again** (isolates forward gameplay): 48,233 games/sec, 4 turns/game, 7.3KB/game, **1,831 bytes/turn**

*Old School decks (realistic 32-41 turn games):*
- **Mono Black vs The Deck**: 1,481 games/sec, 32 turns/game, 824KB/game, 25.7KB/turn
- **White Weenie Mirror**: 1,070 games/sec, 41 turns/game, 1.21MB/game, 29.6KB/turn
- **Jeskai Aggro vs Troll Disk**: 1,139 games/sec, 39 turns/game, 1.22MB/game, 31.4KB/turn

**Latest DHAT heap profiling (2025-11-04_#709, 100 iterations rewind+replay - will refresh after commit):**

Total allocations: 1.13 MB in 26,328 blocks (-39% from baseline 1.86 MB!)
Top hotspots:
1. GameState::advance_step - 150 KB (12.9%) - RNG serialization (FIXED, see mtg-437f88)
2. GameLoop::get_available_spell_abilities - 51.3 KB (4.4%) - helper function allocations
3. Allocator overhead entries (~7-8% each, expected)

**Major wins achieved:**
- ✅ ManaEngine dynamic allocation: 600KB → 0KB (eliminated from top 20!)
- ✅ ManaEngine::update reserve: 70KB → 0KB (eliminated from top 20!)
- ✅ GameLoop abilities buffer: 89KB → 51KB (-43% reduction)
- ✅ RNG serialization: JSON→bincode, 152→56 bytes per turn (63% reduction, 96 bytes/turn saved)
- **Total reduction: From 1.86 MB baseline to ~1.05 MB (est. -44% after RNG fix)**

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

**Low priority (remaining allocations):**
- GameLoop::get_available_spell_abilities helper allocations - 51KB (4.4%)
  - get_lands_in_hand, get_castable_spells return Vecs
  - Would require more API refactoring for modest gains
- Card loading string clones (acceptable one-time cost)
- UndoLog growth (43KB, necessary for rewind functionality)
- Allocator overhead (expected, unavoidable)

**Future considerations:**
- mtg-13: Arena allocation for per-turn temporaries
- mtg-14: Object pools for reusable objects
- mtg-15: Compile-time feature flags for profiling modes

**Optimization status: Excellent!**
We've achieved a ~44% reduction in total allocations (1.86MB → ~1.05MB estimated).
Remaining hotspots are all below 5% and require extensive API refactoring for diminishing returns.

See OPTIMIZATION.md for detailed patterns and profiling methodology.

---
**Updated 2025-11-04_#709(62bb5fd)** - RNG serialization bincode optimization
- Switched RNG serialization from JSON to bincode in advance_step
- JSON: 152 bytes → bincode: 56 bytes (63.2% reduction, fixed size)
- Saves 96 bytes per turn (~8% of advance_step allocations, 150KB→54KB expected)
- All tests passing (408 tests including new rng_serialization_test)
- **Cumulative achievement: ~810 KB eliminated across all optimization work!**
