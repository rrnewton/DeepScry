# Design: Late-Binding CardID ⟺ CardName Architecture

**Status:** DESIGN DOCUMENT
**Date:** 2026-01-05

## Executive Summary

This document describes a major architectural change to make CardIDs **public and shared** between server and all clients, while **deferring the binding** from CardID to CardName until reveal time. This simplifies the network sync model, eliminates hidden card counting, and makes reveals proper undoable game actions.

## Core Concept

### Current Architecture (Complex)
```
Server: CardID 5 = "Lightning Bolt" (known at deck creation)
P1 (owner): CardID 5 = "Lightning Bolt" (knows own cards)
P2 (opponent): CardID ??? (doesn't know until revealed)
              └── Uses hidden_card_count, LibraryMode::Remote, pending_reveals
```

### New Architecture (Simple)
```
Server: CardID 5 = "Lightning Bolt" (bound at reveal)
P1 (owner): CardID 5 = "Lightning Bolt" (bound at reveal)
P2 (opponent): CardID 5 = ??? (slot reserved, name unknown)
              └── All know CardID 5 is in P1's hand, just not what it IS
```

## Key Design Decisions

### 1. Deterministic CardID Assignment

CardIDs are assigned **positionally** in deck order, NOT by card identity:

```
P1's 40-card deck: CardID 0..39   (top to bottom)
P2's 40-card deck: CardID 40..79  (top to bottom)
```

This is PUBLIC information - everyone knows:
- CardID 0 is top of P1's library
- CardID 5 is 6th card from top of P1's library
- After shuffling, order is deterministic from seed

**What's hidden:** The mapping CardID → CardName until revealed.

### 2. RevealCard as Undoable GameAction

```rust
/// Reveal a card's identity (CardID ⟺ CardName binding)
///
/// Logged by ALL players for EVERY reveal, but with different content:
/// - Players who learn the identity: name = Some("Lightning Bolt")
/// - Players who don't learn: name = None (placeholder/dummy reveal)
///
/// This keeps action_count synchronized across all clients while
/// maintaining information asymmetry.
RevealCard {
    card_id: CardId,
    /// The revealed name, or None for players who don't learn it
    name: Option<String>,
},
```

**Undo semantics:**
- Forward: Insert CardInstance into EntityStore at card_id slot
- Backward: Remove CardInstance (set slot back to None)

### 3. Viewer-Specific Action Logs

Each player's undo log may differ in the `name` field of RevealCard:

```
Server log:   RevealCard { card_id: 5, name: Some("Lightning Bolt") }
P1 (owner):   RevealCard { card_id: 5, name: Some("Lightning Bolt") }
P2 (opponent): RevealCard { card_id: 5, name: None }  // Dummy - keeps count in sync
```

When P2 later sees the card (cast, discard to graveyard), they receive another RevealCard that fills in the name.

**Note:** Undo log is explicitly EXCLUDED from state hashing, so this asymmetry doesn't break hash sync.

### 4. EntityStore Changes

```rust
impl<T> EntityStore<T> {
    /// Reserve a slot for an entity that will be revealed later
    ///
    /// Called during game initialization to pre-allocate CardIDs.
    /// The slot remains None until insert() is called with the revealed entity.
    pub fn reserve(&mut self, id: EntityId<T>) {
        let idx = id.as_u32() as usize;
        if idx >= self.entities.len() {
            self.entities.resize_with(idx + 1, || None);
        }
        // Don't check if occupied - we're just ensuring capacity
    }

    /// Check if a slot is revealed (Some) vs reserved-but-unknown (None)
    pub fn is_revealed(&self, id: EntityId<T>) -> bool {
        self.entities.get(id.as_u32() as usize)
            .map(|opt| opt.is_some())
            .unwrap_or(false)
    }

    /// Clear a slot back to None (for undo of RevealCard)
    ///
    /// Returns the removed entity if present.
    pub fn clear(&mut self, id: EntityId<T>) -> Option<T> {
        let idx = id.as_u32() as usize;
        if idx < self.entities.len() {
            self.entities[idx].take()
        } else {
            None
        }
    }
}
```

### 5. Eliminate LibraryMode and hidden_card_count

**Remove entirely:**
- `LibraryMode` enum (Local/Remote)
- `CardZone.library_mode` field
- `CardZone.hidden_card_count` field
- `queue_reveal()`, `pop_reveal()`, `pending_reveals`
- `HiddenDraw`, `HiddenDiscard` action variants

**All zones become uniform:**
```rust
pub struct CardZone {
    pub zone_type: Zone,
    pub owner: PlayerId,
    pub cards: Vec<CardId>,  // Always contains CardIDs
    // No more library_mode or hidden_card_count!
}
```

**Zone contents are always known** (as CardIDs), just not always revealed (as CardNames).

### 6. Draw Flow (Simplified)

**Old flow (network client):**
```
1. Server draws → sends CardRevealed message
2. Client queues reveal in pending_reveals VecDeque
3. Client's draw_card() pops from pending_reveals
4. If no pending, use HiddenDraw to track size
```

**New flow:**
```
1. Server draws → broadcasts RevealCard action (with name)
2. All clients log RevealCard (P1 with name, P2 with None)
3. Server broadcasts MoveCard(card_id, Library → Hand)
4. All clients log MoveCard - they ALL know what CardID moved!
5. Only P1's EntityStore has the card instantiated (P2 has None slot)
```

**Key insight:** Everyone sees the same MoveCard with the same CardID. The difference is whether they have the CardInstance in their EntityStore.

### 7. Discard to Graveyard (Always Reveals)

Graveyard is a PUBLIC zone - discarding always reveals:

```
P1 discards CardID 5:
1. Server: RevealCard { card_id: 5, name: Some("Lightning Bolt") }
2. Server: MoveCard(5, Hand → Graveyard)
3. P2 receives RevealCard with name (now learns the card)
4. P2's EntityStore now has card 5 instantiated
```

This means `HiddenDiscard` is **never needed** - discards always reveal!

## Action Log Design

### What Goes in the Shared Action Log

```rust
enum GameAction {
    // Zone movement (same CardID visible to all players)
    MoveCard { card_id, from_zone, to_zone, owner },

    // Card identity revelation (name differs per viewer)
    RevealCard { card_id, name: Option<String> },

    // Existing actions unchanged
    SetLife { player_id, old_life, new_life },
    Tap { card_id, was_tapped },
    // ... etc
}
```

### Sequencing Invariant

**Option A: Reveal-before-action (orthogonal)**
```
RevealCard { card_id: 5, name: Some("Lightning Bolt") }
CastSpell { card_id: 5, ... }  // Card already revealed
```
- Pro: Clean separation - reveals are explicit
- Pro: Easier undo - unreveal doesn't need to know why
- Con: Extra action for every reveal

**Option B: Action-includes-reveal (combined)**
```
CastSpell { card_id: 5, name: Some("Lightning Bolt"), ... }
```
- Pro: Fewer actions
- Con: Every action type needs optional name field
- Con: Harder to unroll - need to track if this action DID the reveal

**Recommendation: Option A (orthogonal)** - simpler undo semantics.

## State Hashing (Unchanged)

Current design already excludes `undo_log` from hashing. Network hash uses zone SIZES, not contents. This continues to work:

- All players have same CardIDs in same zones
- Zone sizes match
- EntityStore contents may differ (Some vs None)
- But EntityStore is hashed by zone membership, not by card properties

## Migration Strategy

### Phase 1: EntityStore Extensions
1. Add `reserve()` method
2. Add `clear()` method
3. Add `is_revealed()` method
4. Keep existing `insert()` with write-once semantics

### Phase 2: RevealCard Action
1. Add `RevealCard` variant to GameAction
2. Implement forward logic (insert into EntityStore)
3. Implement undo logic (clear from EntityStore)
4. Keep `HiddenDraw`/`HiddenDiscard` for backward compatibility

### Phase 3: Zone Simplification
1. Remove `LibraryMode` enum
2. Remove `hidden_card_count` field
3. Simplify `CardZone` methods
4. Update state hashing for new model

### Phase 4: Network Protocol
1. Update `CardRevealed` message to be RevealCard action broadcast
2. Remove `pending_reveals` VecDeque logic
3. Simplify network client processing

### Phase 5: Cleanup
1. Remove `HiddenDraw`, `HiddenDiscard` action variants
2. Remove backward compatibility shims
3. Update documentation

## Code Locations to Modify

| File | Changes |
|------|---------|
| `entity.rs` | Add `reserve()`, `clear()`, `is_revealed()` |
| `undo.rs` | Add `RevealCard` action + undo logic |
| `zones.rs` | Remove `LibraryMode`, `hidden_card_count` |
| `state.rs` | Simplify `draw_card()`, remove `discard_hidden()` |
| `state_hash.rs` | Update for new zone model |
| `client.rs` | Rewrite reveal processing |
| `server.rs` | Broadcast RevealCard actions |
| `protocol.rs` | Update network message types |

## Benefits

1. **Simpler mental model:** CardIDs are globally shared, only names are hidden
2. **Unified zones:** No more Local/Remote duality
3. **Proper undo:** Reveals are undoable game actions
4. **Cleaner sync:** Everyone logs same actions (with viewer-specific content)
5. **Eliminates:** hidden_card_count, pending_reveals, LibraryMode, HiddenDraw, HiddenDiscard

## Open Questions

### Q1: insert_if_vacant still needed?

Probably not. With reserve + insert:
- `reserve()` during game init for all CardIDs
- `insert()` when RevealCard is processed
- Write-once semantics remain - can only insert into None slot

### Q2: What about searches?

When player searches library:
1. Reveal all matching cards (even if not selected)
2. Selected card moves to hand
3. Library shuffled (new deterministic order from RNG)

The search reveals CardIDs that player can see, even if they don't take them.

### Q3: Scry, look at top N?

1. RevealCard for top N cards (private - only scrying player gets name)
2. Opponent gets RevealCard with None for all N
3. Scry action logs order change
4. No zone change needed

---

## Summary

This redesign makes CardIDs the **stable global identifier** while CardName becomes **viewer-dependent revealed information**. The key insight is that "what CardID is in what zone" is PUBLIC, while "what card IS CardID N" is PRIVATE until revealed.

This eliminates complex hidden-card tracking in favor of simple "is this slot revealed in my EntityStore" checks.
