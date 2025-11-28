---
title: Arena allocation for per-turn temporaries
status: open
priority: 1
issue_type: feature
created_at: 2025-10-26T21:06:34+00:00
updated_at: 2025-11-28T08:34:17.249313023+00:00
---

# Description

## Arena Allocation for Per-Turn, Per-Phase, and Per-Rollout Temporaries

**Priority**: 1 (Performance-critical for MCTS optimization)

## Overview

Use arena allocators (bumpalo or typed-arena) to eliminate heap allocations in hot paths. This issue tracks all allocation sites that are candidates for bump allocation at different scopes.

## Key Insight: Current Architecture is Already Highly Optimized

The undo log uses action-based mutations (IDs, not pointers), making it safe to use bump allocators for temporary vectors without breaking rewind functionality. Only allocate temporary vectors via bump—never persistent game state.

## Scope-Based Arena Strategy

| Scope | Arena Size | Reset Point | Expected Savings |
|-------|-----------|-------------|------------------|
| Per-rollout | ~100 KB | After MCTS simulation | 99% |
| Per-turn | ~10 KB | Turn boundary | 75-85% |
| Per-phase | ~5 KB | Phase boundary | 80-90% |
| Per-priority-round | ~2 KB | After both players pass | 60-70% |

---

## ALLOCATION SITE CHECKLIST

### 🔴 HIGH PRIORITY - Per-Priority-Round (Hot Path)

These allocate every time a player has priority:

- [x] **game_loop/actions.rs:15-46** - `get_available_attacker_creatures()` creates `Vec<CardId>` ✅ (commit 949)
  - Called: Every declare attackers step
  - Fix: Return SmallVec<[CardId; 8]>

- [x] **game_loop/actions.rs:51-69** - `get_available_blocker_creatures()` creates `Vec<CardId>` ✅ (commit 949)
  - Called: Every declare blockers step
  - Fix: Return SmallVec<[CardId; 8]>

- [x] **game_loop/actions.rs:72-74** - `get_current_attackers()` returns `Vec<CardId>` ✅ (commit 946)
  - Delegates to combat.rs:85 which does `.collect()`
  - Fix: Return SmallVec<[CardId; 8]>

- [x] **game_loop/actions.rs:77-91** - `get_lands_in_hand()` creates `Vec<CardId>` ✅ (already refactored)
  - Refactored to `lands_in_hand_iter()` which returns an iterator (zero allocation)

- [x] **game_loop/actions.rs:94-165** - `get_castable_spells()` creates `Vec<CardId>` ✅ (already refactored)
  - Refactored to `push_castable_spells()` which pushes directly to abilities_buffer (zero intermediate allocation)

- [x] **game_loop/actions.rs:168-264** - `get_activatable_abilities()` creates `Vec<(CardId, usize)>` ✅ (already refactored)
  - Refactored to `push_activatable_abilities()` which pushes directly to abilities_buffer (zero intermediate allocation)

- [x] **game_loop/actions.rs:280-331** - `get_available_spell_abilities()` returns `Vec<SpellAbility>` ✅ (already refactored)
  - Uses `std::mem::take()` pattern with `abilities_buffer` - Vec capacity retained across calls

### 🟠 MEDIUM PRIORITY - Per-Turn/Per-Choice

These allocate during controller decisions:

- [x] **random_controller.rs:152** - `available_sources.to_vec()` for shuffling mana sources ✅ (commit 942)
  - Called: Every mana payment choice
  - Fix: SmallVec<[CardId; 8]> inline storage

- [x] **random_controller.rs:260** - `blockers.to_vec()` for damage assignment ✅ (commit 942)
  - Called: Every combat with multi-blocker
  - Fix: SmallVec<[CardId; 4]>

- [x] **random_controller.rs:281** - `hand.to_vec()` for discard choice ✅ (commit 942)
  - Called: Cleanup step if hand > 7
  - Fix: SmallVec<[CardId; 7]>

- [x] **mana_payment.rs:474** - `temp_buffer` Vec in `try_greedy_payment()` ✅ (commit 947)
  - Called: Every complex mana payment
  - Fix: Use SmallVec<[CardId; 8]> internally, copy to output buffer only when needed

- [x] **mana_payment.rs:485-495** - `candidates` Vec in greedy algorithm ✅ (commit 943)
  - Called: Each color being paid
  - Fix: SmallVec<[(usize, u8); 8]> (8 items = 128 bytes inline)

### 🟡 MEDIUM PRIORITY - Heuristic Controller

These allocate during AI decision-making:

- [x] **heuristic_controller.rs:413-418** - `our_creatures: Vec<&Card>` for pump evaluation ✅ (commit 944)
  - Called: Every priority check with pump spells
  - Fix: SmallVec<[&Card; 8]>

- [x] **heuristic_controller.rs:489-505** - `land_plays`, `land_ids` Vecs ✅ (commit 944)
  - Called: During land play decisions
  - Fix: SmallVec<[&SpellAbility; 4]> and SmallVec<[CardId; 4]>

- [x] **heuristic_controller.rs:579-584** - `potential_blockers: Vec<&Card>` ✅ (commit 944)
  - Called: Every combat factor calculation
  - Fix: SmallVec<[&Card; 8]>

- [x] **heuristic_controller.rs:1021, 1475-1521** - Multiple `collect()` calls in blocking logic ✅ (commit 944)
  - Called: During blocker assignment
  - Fix: SmallVec + chained filters to eliminate intermediate allocations

- [x] **heuristic_controller.rs:2048-2228** - Many `Vec<&Card>` for combat simulation ✅ (commit 945)
  - Called: Each attack/block evaluation
  - Fix: SmallVec<[&Card; 8]>, SmallVec<[CardId; 8]> for blockers/attackers

### 🟢 LOWER PRIORITY - Per-Phase/Periodic

- [x] **combat.rs:85** - `get_attackers()` returns `Vec<CardId>` via `.collect()` ✅ (commit 946)
  - Fix: Changed to return SmallVec<[CardId; 8]>

- [x] **combat.rs:100** - `get_blockers_list()` returns `Vec<CardId>` via `.collect()` ✅ (commit 946)
  - Fix: Changed to return SmallVec<[CardId; 8]>

- [x] **game_loop/priority.rs:25-30** - `targets.clone()`, `card_effects.clone()` ✅ (commit 947)
  - Called: Each spell resolution
  - Fix: SmallVec<[CardId; 2]> for targets, defer effects.clone() until logging enabled

- [ ] **mana_engine.rs:229-232** - Vec fields in ManaEngine
  - These are reused via `clear()` - already optimized
  - Note: Capacity retained across calls

### ⚪ LOWEST PRIORITY - Rare/One-Time

- [ ] **state.rs:370** - `milled_cards` Vec in mill operations
  - Rare game action
  
- [ ] **snapshot.rs:237, 266** - `.collect()` in snapshot operations
  - Only during snapshot/resume

---

## Implementation Pattern

```rust
pub struct GameArenas {
    pub turn_arena: Bump,      // 10 KB, reset at turn end
    pub phase_arena: Bump,     // 5 KB, reset at phase end
    pub rollout_arena: Bump,   // 100 KB, reset after MCTS sim
}

// Example refactoring:
fn get_available_attackers<'a>(
    &self, 
    arena: &'a Bump,
    player_id: PlayerId
) -> &'a [CardId] {
    let creatures = arena.alloc_slice_fill_default(expected_count);
    // ... fill creatures ...
    creatures
}
```

---

## Progress Tracking

### Phase 1: SmallVec Quick Wins (No API Changes)
- [x] random_controller.rs: Replace Vec with SmallVec in 3 methods ✅ (commit 942)
- [x] mana_payment.rs: SmallVec for candidates ✅ (commit 943)
- [x] combat.rs: Return SmallVec instead of Vec ✅ (commit 946)

### Phase 2: Arena Infrastructure
- [ ] Add bumpalo dependency
- [ ] Create GameArenas struct
- [ ] Add arena parameters to GameLoop

### Phase 3: Hot Path Refactoring
- [ ] Refactor get_available_* methods to use arena
- [ ] Refactor heuristic_controller combat evaluation
- [ ] Benchmark and verify allocation reduction

---

## Expected Impact

For MCTS with 1000 rollouts:

| Metric | Without Bump | With Bump | Reduction |
|--------|--------------|-----------|-----------|
| Per rollout | ~50-100 KB | Reused 100KB arena | N/A |
| Total allocations | ~50 MB | ~100 KB | 99.8% |
| Allocator contention | High | None (per-thread) | ~10x parallel speedup |

---

## Safety Constraint

**CRITICAL**: Only allocate temporary vectors via bump allocators—never persistent game state. The undo log operates on IDs (safe), not pointers to bump memory (would be unsafe after arena reset).

Safe for bump:
- Temporary query results (attackers, blockers, spells)
- Intermediate calculation buffers
- Controller choice candidates

NOT safe for bump:
- GameState fields
- Undo log entries
- Combat state (persists across phases)

---

Related issues: mtg-2 (optimization tracking), mtg-162 (parallel MCTS bottleneck)

## Progress (2025-11-28)

**Infrastructure complete:**
- ✅ Added `#![feature(allocator_api)]` to lib.rs for nightly Vec<T, A> support
- ✅ Added `bumpalo` with `allocator_api` feature in Cargo.toml
- ✅ Added `pub bump: Bump` to GameState with `#[serde(skip)]`
- ✅ Manual Clone impl for GameState (each clone gets fresh `Bump::new()`)
- ✅ Test demonstrating `Vec::new_in(&game.bump)` works

**Observations:**
- Most allocations found during investigation were "stupid allocations" that should be eliminated rather than arena-allocated
- Refactored get_available_spell_abilities to have zero intermediate allocations (iterator + direct buffer push)
- Remaining candidates for bump allocation:
  - `get_available_attacker_creatures` / `get_available_blocker_creatures` (return sorted Vecs to controller)
  - These happen once per combat phase (less frequent than spell ability queries)

**Commits:**
- 881f9a06: feat(alloc): Add bump allocator to GameState with allocator_api
- cc155429: perf(alloc): Eliminate Vec allocation in get_lands_in_hand
- 7af8fc68: perf(alloc): Eliminate Vec allocations in spell/ability queries
- (commit 942): perf(alloc): Use SmallVec instead of Vec for random_controller shuffles

## Progress (2025-11-28, commit 942)

**SmallVec in random_controller.rs:**
- ✅ `choose_mana_sources_to_pay()`: SmallVec<[CardId; 8]> for shuffling mana sources
- ✅ `choose_damage_assignment_order()`: SmallVec<[CardId; 4]> for blocker ordering
- ✅ `choose_cards_to_discard()`: SmallVec<[CardId; 7]> for discard shuffling

**Benchmark results (2025-11-28_#941):**
- `fresh_games`: -5.3% to -1.3% improvement
- `whiteweenie_mirror/rewind_play_again`: -4.9% to -4.1% improvement
- `jeskai_trolldisk/rewind_play_again`: -4.2% to -1.5% improvement
- Key metric `mem_logging_rewind_play_again`: 2.54M actions/sec, 209.45 bytes/action

## Progress (2025-11-28, commit 943)

**SmallVec in mana_payment.rs:**
- ✅ `try_greedy_payment()` candidates: SmallVec<[(usize, u8); 8]> for mana source candidates

**Benchmark results (2025-11-28_#943):**
- Key metric `mem_logging_rewind_play_again`: 196.31 bytes/action (down from 209.45, **6.3% reduction**)
- Parallel benchmarks: -6% to -11% improvement
- Sequential benchmarks: within noise

## Progress (2025-11-28, commit 944)

**SmallVec in heuristic_controller.rs:**
- ✅ `our_creatures`: SmallVec<[&Card; 8]> for pump spell evaluation
- ✅ `land_plays`: SmallVec<[&SpellAbility; 4]> for land play decisions
- ✅ `land_ids`: SmallVec<[CardId; 4]> for land card IDs
- ✅ `potential_blockers`: SmallVec<[&Card; 8]> for combat factor calculation
- ✅ `opponent_creatures`: SmallVec<[&Card; 8]> for blocking checks
- ✅ `choose_best_removal_target`: Consolidated 4 chained collects into single SmallVec collect

**Benchmark results (2025-11-28_#944):**
- Key metric `mem_logging_rewind_play_again`: 193.56 bytes/action (down from 196.31, cumulative **7.6% reduction** from 209.45)
- Multiple benchmarks showing "Performance has improved"

## Progress (2025-11-28, commit 945)

**SmallVec in heuristic_controller.rs combat simulation (assign_blocks_phase1/2/3):**
- ✅ `remaining_blockers`: SmallVec<[CardId; 8]> - tracks available blockers
- ✅ `attacker_cards`, `attackers_left`: SmallVec<[&Card; 8]> - sorted attacker list
- ✅ `blocker_cards`, various: SmallVec<[&Card; 8]> - available blockers
- ✅ `gang_blocked_attacker_ids`: SmallVec<[CardId; 4]> - gang block tracking
- ✅ `trample_attackers`: SmallVec<[CardId; 4]> - trample detection
- ✅ `current_blockers`: SmallVec<[&Card; 4]> - per-attacker blockers

**Benchmark results (2025-11-28_#945):**
- Key metric `mem_logging_rewind_play_again`: 195.31 bytes/action (within noise of 193.56)
- snapshot_games improved
- All heuristic controller Vec allocations now use SmallVec

## Progress (2025-11-28, commit 946)

**SmallVec return types in combat.rs:**
- ✅ `get_attackers()`: Returns SmallVec<[CardId; 8]> instead of Vec<CardId>
- ✅ `get_blockers_list()`: Returns SmallVec<[CardId; 8]> instead of Vec<CardId>
- ✅ `get_current_attackers()` in actions.rs: Updated to return SmallVec<[CardId; 8]>

**Benchmark results (2025-11-28_#946):**
- Key metric `mem_logging_rewind_play_again`: 195.37 bytes/action (within noise)
- `whiteweenie_mirror/rewind_play_again`: improved
- Phase 1 SmallVec Quick Wins complete!

## Progress (2025-11-28, commit 947)

**Additional SmallVec optimizations:**
- ✅ `mana_payment.rs:try_greedy_payment()`: Use SmallVec<[CardId; 8]> for internal tracking
  - Avoids heap allocation for typical mana payments (up to 8 sources)
  - Only copies to output Vec when caller requests tap order
- ✅ `priority.rs:resolve_top_spell_from_stack()`: SmallVec<[CardId; 2]> for targets
  - Most spells have 0-2 targets, avoids heap allocation
  - Defer `card.effects.clone()` until logging is enabled (skipped during silent mode/replaying)

**Note:** These optimizations reduce allocations in:
- Silent mode (benchmarks): effects.clone() skipped entirely
- Replaying mode: effects.clone() skipped entirely
- Mana payment checks: SmallVec avoids heap allocation for temp tracking

## Progress (2025-11-28, commit 949)

**SmallVec for attacker/blocker creature lists:**
- ✅ `get_available_attacker_creatures()`: Returns SmallVec<[CardId; 8]> instead of Vec<CardId>
- ✅ `get_available_blocker_creatures()`: Returns SmallVec<[CardId; 8]> instead of Vec<CardId>

**All HIGH PRIORITY items from checklist are now complete!**

The entire "Per-Priority-Round (Hot Path)" section is resolved:
- Attacker/blocker queries: SmallVec return types
- Land plays: Iterator pattern (zero allocation)
- Castable spells: Direct buffer push (zero intermediate allocation)
- Activatable abilities: Direct buffer push (zero intermediate allocation)
- Spell abilities: Vec with retained capacity via std::mem::take()
