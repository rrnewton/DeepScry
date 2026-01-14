---
title: 'RFC: Name-based library search protocol for network mode'
status: closed
priority: 3
issue_type: feature
depends_on:
  mtg-secqu: discovered-from
created_at: 2026-01-13T18:49:07.043506364+00:00
updated_at: 2026-01-14T22:34:18.154727673+00:00
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
6. Server sends: LibraryReordered with shuffled library order

### Why This Works
- Client never sees full library contents (just unique names)
- Only the chosen card's CardId is revealed
- Post-shuffle, library order is synced via LibraryReordered message

## Implementation Status

### Completed
- [x] Added LibrarySearchByName variant to ChoiceType enum
- [x] Added LibraryReordered message type to ServerMessage
- [x] Modified NetworkController::choose_from_library() for name-based selection
- [x] Removed deprecated LibrarySearch ChoiceType
- [x] Added ShuffleLibrary to GameAction enum for undo log tracking
- [x] Modified shuffle_library() to log action with previous order
- [x] Implemented undo logic for ShuffleLibrary in both undo.rs and state.rs
- [x] Added maybe_reveal_to_all() call before moving searched card
- [x] Fixed reveal collection to use actual card owner (not controller's player ID)
- [x] Added CardRevealed forwarding to local controller for library search tracking
- [x] **Server sends initial LibraryReordered after GameStarted**
- [x] **Server collects and sends LibraryReordered after shuffles**
- [x] **Client applies LibraryReordered to shadow state**
- [x] **Tutor deck (Evolving Wilds, Demonic Tutor) works in network mode**

### Testing
Successfully tested with tutor_test.dck deck:
- Game runs through multiple turns with library searches
- No reveal validation failures or desync after shuffles
- Both fetch lands (Evolving Wilds) and unrestricted tutors (Demonic Tutor) work

## Files Changed
- mtg-engine/src/network/protocol.rs - LibrarySearchByName, LibraryReordered
- mtg-engine/src/network/controller.rs - Name-based selection, library reorder collection
- mtg-engine/src/network/client.rs - LibraryReordered handling during init and gameplay
- mtg-engine/src/network/server.rs - Send initial and post-shuffle LibraryReordered
- mtg-engine/src/game/game_loop/mod.rs - LibraryOrderPusher callback
- mtg-engine/src/game/game_loop/priority.rs - push_library_order after shuffle
- mtg-engine/src/undo.rs - ShuffleLibrary GameAction variant

Dependencies:
  mtg-secqu (discovered-from)
