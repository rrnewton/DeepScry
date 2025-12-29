---
title: Fix entity ID mismatch between server and clients in network games
status: open
priority: 2
issue_type: task
created_at: 2025-12-29T02:36:24.295147240+00:00
updated_at: 2025-12-29T02:36:24.295147240+00:00
---

# Description

## Problem

Network games deadlock because the server and client have different entity IDs for the same cards.

### Evidence from logs

Server at action_count=38:
```
Priority check: player 1 has 1 available abilities at action_count=38: ["CastSpell { card_id: 74 }"]
Server NetworkController 1: sending ChoiceRequest #2 (action_count=38, type=Priority { available_count: 1 })
```

Client at action_count=38:
```
Priority check: player 1 has 0 available abilities, action_count=38
```

The server sees Spider-Suit (card 74) as castable, but the client sees 0 available abilities because its card 74 is a different card.

### Root Cause

In `mtg-engine/src/network/client.rs` lines 491-496, the client creates its own game state with locally-assigned entity IDs:

```rust
let initializer = GameInitializer::new(card_db);
let mut game = initializer
    .init_game(p1_name, p1_deck, p2_name, p2_deck, starting_life)
    .await?;
```

The server also creates a game with its own entity IDs. These IDs don't match because:
1. Entity ID counters start from different values
2. Cards may be created in different orders

### Proposed Fix

Option A: Have clients create cards on-demand using server's IDs from CardRevealed messages
Option B: Ensure both server and client use identical deterministic ID assignment (same counter, same order)

Option B is cleaner - the server should send the starting entity ID counter value to clients during game initialization, and both should use identical deterministic shuffling.

## Related

This is blocking mtg-037fw (4-way network gamelog equivalence test).
