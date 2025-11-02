---
title: Optimization and performance tracking
status: open
priority: 1
issue_type: epic
labels:
- tracking
created_at: 2025-10-26T21:06:34+00:00
updated_at: 2025-11-02T22:55:41.496434741+00:00
---

# Description

Track performance optimization work for MTG Forge Rust.

**Current performance as of 2025-11-02_#593(d071ccf):**

*Simple deck (simple_bolt.dck):*
- **Fresh Mode**: 5,431 games/sec, avg 7 turns/game, 244KB/game, 34.8KB/turn
- **Snapshot Mode**: 19,224 games/sec (3.5x faster via clone)
- **Rewind Mode**: 196,721 games/sec (36x faster via undo)

*Old School decks (realistic 30-56 turn games):*
- **Mono Black vs The Deck**: 1,476 games/sec, 32 turns/game, 811KB/game, 25.3KB/turn
- **White Weenie Mirror**: 994 games/sec, 56 turns/game, 1.25MB/game, 22.3KB/turn
- **Jeskai Aggro vs Troll Disk**: 1,117 games/sec, 39 turns/game, 1.26MB/game, 32.4KB/turn

**Recent optimization impact (d071ccf):**
- Simple deck: **20.3% allocation reduction** (305KB → 244KB per game)
- Old school decks: **29-39% allocation reduction** (longer games benefit more)
- Speed improvement: **15-16% faster** for old school matchups
- Key: Reusable ManaEngine in GameLoop eliminates repeated allocations

**Completed optimizations:**
- ✅ mtg-6: Logging allocations (conditional compilation added)
- ✅ mtg-10: Vec reallocations in game loop (SmallVec + fixed arrays)
- ✅ mtg-7: CardDatabase.get_card() returns Arc<CardDefinition>
- ✅ mtg-8: GameStateView already uses borrowing, not cloning
- ✅ mtg-9: CardName and PlayerName use Arc<str>
- ✅ mtg-12: Mana pool calculation optimization (already resolved)
- ✅ mtg-11: Zone transfer operations (investigated, already optimal)
- ✅ mtg-934c9c: **ManaEngine allocation hotspot - MAJOR WIN**
  - Stored single reusable ManaEngine in GameLoop
  - Added capacity pre-allocation (reserve 10/5/15)
  - 20-39% allocation reduction, 15-16% speed improvement

**High priority open issues:**
- (None currently)

**Medium priority:**
- (None currently)

**Future considerations:**
- mtg-13: Arena allocation for per-turn temporaries
- mtg-14: Object pools for reusable objects
- mtg-15: Compile-time feature flags for profiling modes

See OPTIMIZATION.md for detailed analysis and profiling methodology.

---
**Updated 2025-11-02_#593(d071ccf)**
- ManaEngine refactor completed with excellent results
- Allocation reduction: 20-39% depending on game complexity
- Performance improvement: 15-16% for realistic decks
- All 404 tests passing
