---
title: Network state hashing (HashMode::Network)
status: closed
priority: 2
issue_type: task
depends_on:
  mtg-to96y: parent-child
created_at: 2025-12-05T17:57:26.062246862+00:00
updated_at: 2025-12-05T18:09:39.325192214+00:00
---

# Description

## Network State Hash

Extend state_hash.rs to support network verification hashes that exclude hidden information.

## Tasks

- [ ] Add `HashMode` enum (Replay, UndoTest, Network)
- [ ] Add `NETWORK_EXCLUDED_FIELDS` constant (rng, library contents, hand contents)
- [ ] Implement `compute_state_hash_with_mode(game, mode)` 
- [ ] Implement `inject_zone_sizes()` to add hand/library SIZES after stripping contents
- [ ] Add `compute_network_state_hash(game)` convenience wrapper
- [ ] Unit tests verifying hash excludes hidden info but includes sizes
- [ ] Verify existing replay/undo tests still pass

## Key Principle

Hash includes only PUBLIC information:
- Battlefield, stack, graveyard, exile (full contents)
- Life totals, turn/step info
- Hand and library SIZES (not contents)

## Reference

See `ai_docs/NETWORKING_DESIGN_PLAN.md` Section 1.2
