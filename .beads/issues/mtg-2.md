---
title: Optimization and performance tracking
status: open
priority: 1
issue_type: epic
labels:
- tracking
created_at: 2025-10-26T21:06:34+00:00
updated_at: 2025-11-04T11:50:22.458980726+00:00
---

# Description

Track performance optimization work for MTG Forge Rust.

**Current performance as of 2025-11-03_#597(6e47d7d):**

*Simple deck (simple_bolt.dck):*
- **Fresh Mode**: 5,502 games/sec, avg 7 turns/game, 244KB/game, 34.8KB/turn
- **Snapshot Mode**: 19,291 games/sec (3.5x faster via clone)
- **Rewind Mode**: 202,583 games/sec (36.8x faster via undo)

*Old School decks (realistic 30-56 turn games):*
- **Mono Black vs The Deck**: 1,507 games/sec, 32 turns/game, 811KB/game, 25.3KB/turn
- **White Weenie Mirror**: 1,029 games/sec, 56 turns/game, 1.25MB/game, 22.3KB/turn
- **Jeskai Aggro vs Troll Disk**: 1,128 games/sec, 39 turns/game, 1.26MB/game, 32.4KB/turn

**Latest heap profiling (2025-11-03_#597, 1000 games, seed 42):**

Total allocations: 1,267,713 across 1000 games (~1,268 per game)
Temporary allocations: 484,954 (38% of total)

**Top allocation sites by call count:**

1. **Card instantiation** - 240,000 calls (19%)
   - src/loader/game_init.rs:75 - `card_def.instantiate(card_id, player_id)`
   - One-time cost during deck loading (60 cards x 2 players x 2 decks)
   - Priority: Low (not per-turn)

2. **Oracle text cloning** - 120,000 calls (9%)
   - src/loader/card.rs:168 - `card.text = self.oracle.clone()`
   - String cloning during card instantiation
   - Priority: Low (one-time setup cost)

3. **Subtype wrapper allocations** - 80,000 calls (6%)
   - src/core/types.rs:14 - `Subtype(String)` wrapper
   - Called during card creation
   - Priority: Low (setup cost)

4. **AI spell selection** - 55,000 calls (4%)
   - src/game/game_loop.rs:2147 - `controller.choose_spell_ability_to_play()`
   - Allocations in AI decision-making
   - Priority: Medium (per-turn, but complex tradeoff)

5. **Mana payment calculations** - 48,000 calls (4%)
   - src/game/mana_payment.rs:348 - `tap_color()` function
   - Vec allocations during cost payment
   - Priority: Medium (per-spell cast)

**Key insights:**
- Most allocations (19%+9%+6% = 34%) are one-time setup costs (card loading)
- Per-turn allocations (AI, mana payment) are only ~8% of total
- Logging allocations are now minimal (verbose-logging feature working!)
- No pathological allocation patterns detected

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

**High priority open issues:**
- (None currently - all major hotspots addressed)

**Medium priority:**
- Mana payment Vec allocations (mtg-payment-vecs) - 48K calls
- AI decision allocations (mtg-ai-allocs) - 55K calls
  
**Low priority (setup costs):**
- Card loading string clones (acceptable one-time cost)

**Future considerations:**
- mtg-13: Arena allocation for per-turn temporaries
- mtg-14: Object pools for reusable objects
- mtg-15: Compile-time feature flags for profiling modes

See OPTIMIZATION.md for detailed patterns and profiling methodology.

---
**Updated 2025-11-03_#597(6e47d7d)**
- Fresh heap profiling with 1000 games completed
- ManaResolver Box elimination: minimal allocation impact, 3-7% speed improvement
- Key finding: Most allocations are one-time setup (34%), not hot-path
- Removed dated profiling results from OPTIMIZATION.md, moved here
