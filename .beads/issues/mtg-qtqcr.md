---
title: Late-binding CardID<=>CardName architecture
status: open
priority: 1
issue_type: task
labels:
- epic
- network
- architecture
created_at: 2026-01-05T23:37:07.275378052+00:00
updated_at: 2026-01-05T23:37:07.275378052+00:00
---

# Description

## Epic: Late-Binding CardID ⟺ CardName Architecture

**Design Doc:** `ai_docs/DESIGN_late_binding_cardid.md`

## Overview

Major architectural change to make CardIDs **public and shared** between server and all clients, while **deferring the binding** from CardID to CardName until reveal time.

### Core Concept
- CardIDs are assigned positionally (0..N for deck1, N+1..M for deck2)
- Everyone knows "CardID 5 is in P1's hand"
- Only revealed players know "CardID 5 is Lightning Bolt"
- RevealCard becomes an undoable GameAction

## Implementation Phases

### Phase 1: EntityStore Extensions ✅ COMPLETE
- [x] Add `reserve(id)` method - pre-allocate slot
- [x] Add `reserve_range(start, count)` method - batch reservation
- [x] Add `clear(id)` method - set slot back to None (for undo)
- [x] Add `is_revealed(id)` method - check if slot has entity
- [x] Keep existing `insert()` with write-once semantics
- [x] Add 5 comprehensive unit tests

### Phase 2: RevealCard GameAction ✅ COMPLETE
- [x] Add `RevealCard { card_id, name, card }` variant
- [x] Implement undo logic in `undo.rs` (clear from EntityStore)
- [x] Implement undo logic in `state.rs` (parallel match arm)
- [x] Add Display impl for RevealCard
- [x] Add 5 unit tests for RevealCard
- [x] Keep HiddenDraw/HiddenDiscard for backward compat initially

### Phase 3: Network Protocol Updates (swapped with Zone Simplification)
**Note:** Phase 3 and 4 were swapped because zone simplification depends on
the network protocol being updated first. The zones use LibraryMode/hidden_card_count
to support the current reveal-via-pending-buffer model; we need the new
RevealCard-based protocol working before we can simplify zones.

- [ ] Pre-allocate deterministic CardIDs at game start (0..N for P1, N+1..M for P2)
- [ ] Update server to broadcast deck CardIDs to clients in GameStart
- [ ] Update client to reserve CardID slots when receiving GameStart
- [ ] Update CardRevealed message to include RevealCard action semantics
- [ ] Transition client from pending_reveals buffer to direct EntityStore insert

### Phase 4: Zone Simplification (moved here from Phase 3)
**Requires:** Phase 3 network protocol complete

- [ ] Remove `LibraryMode` enum entirely
- [ ] Remove `hidden_card_count` field from CardZone
- [ ] Simplify CardZone methods (len, draw_top, etc.)
- [ ] Update state hashing for new model

### Phase 5: Cleanup
- [ ] Remove HiddenDraw, HiddenDiscard action variants
- [ ] Remove insert_if_vacant (no longer needed)
- [ ] Remove backward compatibility shims
- [ ] Update documentation

## Key Design Decisions

1. **Reveal-before-action (Option A)**: RevealCard precedes any action that uses the card
2. **Viewer-specific action logs**: RevealCard has `name: Some(...)` or `None` per viewer
3. **Undo log excluded from hash**: Already the case, so asymmetric logs work

## Files to Modify

| File | Changes |
|------|---------|
| `entity.rs` | Add reserve(), clear(), is_revealed() |
| `undo.rs` | Add RevealCard action + undo logic |
| `zones.rs` | Remove LibraryMode, hidden_card_count |
| `state.rs` | Simplify draw_card(), remove discard_hidden() |
| `state_hash.rs` | Update for new zone model |
| `client.rs` | Rewrite reveal processing |
| `server.rs` | Broadcast RevealCard actions, pre-allocate IDs |
| `protocol.rs` | Update network message types |

## Benefits
- Simpler mental model
- Unified zones (no Local/Remote duality)
- Proper undo for reveals
- Eliminates: hidden_card_count, pending_reveals, LibraryMode, HiddenDraw, HiddenDiscard
