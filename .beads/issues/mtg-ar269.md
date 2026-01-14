---
title: 'RFC: Name-based library search protocol for network mode'
status: open
priority: 3
issue_type: feature
depends_on:
  mtg-secqu: discovered-from
created_at: 2026-01-13T18:49:07.043506364+00:00
updated_at: 2026-01-13T18:49:07.043506364+00:00
---

# Description

## Summary

Implemented the name-based library search protocol. This addresses the issue where tutoring effects in network mode would require revealing CardIds that shouldn't be visible to clients.

## Design

**Core insight**: Tutors care about card NAMES, not card IDs.

### Protocol Flow
1. Server filters library for matching cards, extracts unique names
2. Server sends: ChoiceRequest { type: LibrarySearchByName, options: ["Decline", "Island", "Swamp"] }
3. Client picks "Island" (index 1)
4. Server picks a specific CardId with name "Island"
5. Server sends: CardRevealed for the chosen card only

### Why This Works
- Client never sees full library contents (just unique names)
- Only the chosen card's CardId is revealed
- Post-shuffle, library is just CardIds again (identities unknown)

## Implementation Status

### Completed
- [x] Added LibrarySearchByName variant to ChoiceType enum
- [x] Added LibraryReordered message type to ServerMessage
- [x] Modified NetworkController::choose_from_library() for name-based selection
- [x] Updated client.rs to handle LibraryReordered (stub)
- [x] Removed deprecated LibrarySearch ChoiceType
- [x] Added ShuffleLibrary to GameAction enum for undo log tracking
- [x] Modified shuffle_library() to log action with previous order
- [x] Implemented undo logic for ShuffleLibrary in both undo.rs and state.rs

### Deferred
- [ ] Emit LibraryReordered from server after library shuffle
- [ ] Client shadow state library zone update from LibraryReordered

## Files Changed
- mtg-engine/src/network/protocol.rs - LibrarySearchByName, LibraryReordered, removed deprecated LibrarySearch
- mtg-engine/src/network/controller.rs - Name-based selection logic
- mtg-engine/src/network/client.rs - LibraryReordered handler stub
- mtg-engine/src/undo.rs - ShuffleLibrary GameAction variant
- mtg-engine/src/game/state.rs - shuffle_library() logging, undo support
