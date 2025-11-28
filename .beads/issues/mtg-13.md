---
title: Arena allocation for per-turn temporaries
status: open
priority: 4
issue_type: feature
created_at: "2025-10-26T21:06:34Z"
updated_at: "2025-10-26T21:06:34Z"
---

# Description

Use arena allocators (bumpalo or typed-arena) for per-turn allocations.
Benefits: faster allocation (pointer increment), bulk deallocation, better cache locality.

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
