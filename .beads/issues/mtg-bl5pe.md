---
title: 'Engine refactoring: LibraryMode and RemoteLibrary'
status: open
priority: 2
issue_type: task
depends_on:
  mtg-to96y: parent-child
created_at: 2025-12-05T17:57:37.785380850+00:00
updated_at: 2025-12-05T17:57:37.785380850+00:00
---

# Description

## Remote Library Abstraction

Refactor the engine to support remote libraries where contents are revealed incrementally.

## Tasks

- [x] Create `LibraryMode` enum (Local, Remote)
- [x] Remote mode has: size counter, pending_reveals buffer (VecDeque)
- [x] Modify `CardZone` to optionally use LibraryMode for library zones
- [x] Update `draw_top()` to handle both modes
- [x] Add `queue_reveal(card_id)` for remote mode
- [x] Add `add_to_top()`, `add_to_bottom()`, `peek_top()` for remote mode
- [x] Unit tests for both modes (8 new tests)
- [ ] Update `draw_card()` in GameState to work with remote libraries
- [ ] Handle tutor/search effects with remote libraries

## Key Behavior

**Local mode** (server): `draw_top()` pops from cards vec
**Remote mode** (client): `draw_top()` pops from pending_reveals buffer; panics if buffer empty (sync error)

## Reference

See `ai_docs/NETWORKING_DESIGN_PLAN.md` Part 2
