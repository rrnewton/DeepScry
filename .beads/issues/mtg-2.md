---
title: Optimization and performance tracking
status: open
priority: 1
issue_type: epic
labels:
- tracking
created_at: 2025-10-26T21:06:34+00:00
updated_at: 2025-11-02T22:35:31.491960991+00:00
---

# Description

Track performance optimization work for MTG Forge Rust.

**Current performance as of 2025-11-02_#584(1f5f2e5):**

*Simple deck (simple_bolt.dck):*
- **Fresh Mode**: 5,188 games/sec, avg 7 turns/game, 305KB/game, 43.7KB/turn
- **Snapshot Mode**: 16,072 games/sec (3.1x faster via clone)
- **Rewind Mode**: 195,276 games/sec (37.6x faster via undo)

*Old School decks (realistic 30-56 turn games):*
- **Mono Black vs The Deck**: 1,262 games/sec, 32 turns/game, 1.2MB/game, 39.0KB/turn
- **White Weenie Mirror**: 811 games/sec, 56 turns/game, 2.0MB/game, 36.5KB/turn
- **Jeskai Aggro vs Troll Disk**: 939 games/sec, 39 turns/game, 1.8MB/game, 45.8KB/turn

**Key insight**: Allocation per turn is stable (36-46 KB/turn) across all deck types, demonstrating consistent per-turn overhead.

**Completed optimizations:**
- ✅ mtg-6: Logging allocations (conditional compilation added)
- ✅ mtg-10: Vec reallocations in game loop (SmallVec + fixed arrays)
- ✅ mtg-7: CardDatabase.get_card() returns Arc<CardDefinition>
- ✅ mtg-8: GameStateView already uses borrowing, not cloning
- ✅ mtg-9: CardName and PlayerName use Arc<str>
- ✅ mtg-12: Mana pool calculation optimization (already resolved)
- ✅ mtg-11: Zone transfer operations (investigated, already optimal)

**High priority open issues:**
- mtg-934c9c: Reduce allocations in ManaEngine::update() hot path
  - Heaptrack profiling identified this as primary allocation hotspot
  - 158 Vec::push calls per 100 games
  - Called multiple times per priority round
  - Proposal: Pre-allocate vector capacity based on typical battlefield sizes

**Medium priority:**
- (None currently)

**Future considerations:**
- mtg-13: Arena allocation for per-turn temporaries
- mtg-14: Object pools for reusable objects
- mtg-15: Compile-time feature flags for profiling modes

See OPTIMIZATION.md for detailed analysis and profiling methodology.

---
**Updated 2025-11-02_#584(1f5f2e5)**
- Added realistic old_school deck benchmarks
- Performed heaptrack profiling to identify allocation hotspots
- Created mtg-934c9c for ManaEngine optimization
- Bytes/turn is stable across deck complexity (36-46 KB/turn)
