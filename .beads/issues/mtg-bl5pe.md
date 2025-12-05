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

- [ ] Create `LibraryMode` enum (Local, Remote)
- [ ] Remote mode has: size counter, pending_reveals buffer (VecDeque)
- [ ] Modify `CardZone` to optionally use LibraryMode for library zones
- [ ] Update `draw_top()` to handle both modes
- [ ] Add `queue_reveal(card_id)` for remote mode
- [ ] Update `draw_card()` in GameState to work with remote libraries
- [ ] Handle tutor/search effects with remote libraries
- [ ] Unit tests for both modes

## Key Behavior

**Local mode** (server): `draw_top()` pops from cards vec
**Remote mode** (client): `draw_top()` pops from pending_reveals buffer; panics if buffer empty (sync error)

## Reference

See `ai_docs/NETWORKING_DESIGN_PLAN.md` Part 2
