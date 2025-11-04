---
title: Optimization and performance tracking
status: open
priority: 1
issue_type: epic
labels:
- tracking
created_at: 2025-10-26T21:06:34+00:00
updated_at: 2025-11-04T19:05:47.234579182+00:00
---

# Description

## Description

Track performance optimization work for MTG Forge Rust.

**Current performance as of 2025-11-04_#706(4901478):**

*Simple deck (simple_bolt.dck):*
- **Fresh Mode**: 5,658 games/sec, avg 7 turns/game, 236KB/game, 33.7KB/turn
- **Snapshot Mode**: 19,656 games/sec (3.5x faster via clone)
- **Rewind Mode**: 201,562 games/sec (35.6x faster via undo)
- **Rewind + Play Again** (isolates forward gameplay): 48,928 games/sec, 4 turns/game, 7.9KB/game, **1,974 bytes/turn**

*Old School decks (realistic 32-41 turn games):*
- **Mono Black vs The Deck**: 1,492 games/sec, 32 turns/game, 830KB/game, 25.9KB/turn
- **White Weenie Mirror**: 1,080 games/sec, 41 turns/game, 1.22MB/game, 29.8KB/turn
- **Jeskai Aggro vs Troll Disk**: 1,146 games/sec, 39 turns/game, 1.22MB/game, 31.4KB/turn

**Latest DHAT heap profiling (2025-11-04_#706, 100 iterations rewind+replay):**

Total allocations: 1.19 MB in 26,429 blocks (-36% from previous)
Top hotspots:
1. GameState::advance_step - 150 KB (12.3%) - RNG serialization (see mtg-437f88)
2. ManaEngine::update - 70.3 KB (5.8%) - remaining after dynamic allocation fix
3. GameLoop::get_available_spell_abilities - 51.3 KB (4.2%)
4. GameLoop::get_available_spell_abilities (abilities vec) - 38 KB (3.1%)

**Completed optimizations:**
- ✅ mtg-6: Logging allocations (conditional compilation added)
- ✅ mtg-10: Vec reallocations in game loop (SmallVec + fixed arrays)
- ✅ mtg-7: CardDatabase.get_card() returns Arc<CardDefinition>
- ✅ mtg-8: GameStateView already uses borrowing, not cloning
- ✅ mtg-9: CardName and PlayerName use Arc<str>
- ✅ mtg-12: Mana pool calculation optimization (already resolved)
- ✅ mtg-11: Zone transfer operations (investigated, already optimal)
- ✅ mtg-120: ManaEngine allocation hotspot - MAJOR WIN
  - Stored single reusable ManaEngine in GameLoop
  - Added capacity pre-allocation (reserve 10/5/15)
  - 20-39% allocation reduction, 15-16% speed improvement
- ✅ mtg-current: ManaResolver Box elimination
  - Store both resolvers directly, switch with bool flag
  - Minimal allocation impact (~2% measurement variance)
  - 3-7% speed improvement from reduced indirection
- ✅ mtg-payment-vecs: Mana payment Vec elimination - MAJOR WIN (2025-11-04_#704)
  - Eliminated 1.4M Vec allocations from SimpleManaResolver::check_payment
  - Changed API to use output buffer pattern instead of returning Vec
  - Performance: 85% faster (115µs → 17µs), allocation hotspot completely eliminated
  - All 406 tests passing
- ✅ mtg-mana-engine-dynamic: ManaEngine dynamic allocation elimination - MAJOR WIN (2025-11-04_#706)
  - Refactored cast_spell_8_step to accept &ManaEngine instead of closure callback
  - Pre-compute mana_engine in GameLoop, eliminating repeated allocations
  - DHAT results: 600KB → 70KB (-88% reduction in ManaEngine::update hotspot)
  - Total allocations: 1.86MB → 1.19MB (-36% overall)
  - Performance improvements: 3-24% faster across all benchmarks
  - All 276 unit + 8 integration tests passing

**High priority open issues:**
- (None currently - all major hotspots below 13%)

**Medium priority (current DHAT profiling results):**
- **GameState::advance_step RNG serialization** - 150KB (12.3%)
  - Issue: mtg-437f88
  - Location: src/game/state.rs:456 (serde_json::to_vec for RNG state)
  - Root cause: ChaCha12Rng serialization via JSON, stored in undo log
  - Fix: Switch to more compact RNG (PCG/Xoshiro) with fixed-size state
  - Estimated impact: 12% allocation reduction (but minimal perf impact)

- **GameLoop::get_available_spell_abilities** - 51.3KB + 38KB = 89.3KB (7.3% combined)
  - Locations: game_loop.rs:3034 and 3025
  - Fix: Store reusable Vec buffer in GameLoop for ability lists
  - Estimated impact: 7% allocation reduction

- **ManaEngine::update remaining allocations** - 70.3KB (5.8%)
  - Location: src/game/mana_engine.rs:264
  - Already improved from 600KB, but still allocating on each call
  - Fix: Store reusable Vec buffer in ManaEngine (same pattern as mana payment)
  - Estimated impact: 5% allocation reduction

**Low priority (setup costs or minor):**
- Card loading string clones (acceptable one-time cost)
- UndoLog growth (43KB, but necessary for rewind functionality)
- Various allocator overhead entries

**Future considerations:**
- mtg-13: Arena allocation for per-turn temporaries
- mtg-14: Object pools for reusable objects
- mtg-15: Compile-time feature flags for profiling modes

See OPTIMIZATION.md for detailed patterns and profiling methodology.

---
**Updated 2025-11-04_#706(4901478)** - ManaEngine dynamic allocation elimination
- MAJOR WIN: Eliminated 88% of ManaEngine::update allocations (600KB → 70KB)
- Refactored cast_spell_8_step API: closure callback → &ManaEngine parameter
- Total allocation reduction: 36% (1.86MB → 1.19MB in DHAT profiling)
- Performance improvements: 3-24% faster across all benchmarks
- Fresh mode throughput: +6.7% (5,301 → 5,658 games/sec)
- Created mtg-437f88 for RNG serialization optimization
- Ready to proceed with GameLoop buffer optimizations
