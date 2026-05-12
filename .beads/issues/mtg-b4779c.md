---
title: 'Cleanup step: non-active player incorrectly forced to discard'
status: closed
priority: 2
issue_type: task
labels:
- single-card
created_at: 2026-04-03T21:22:30.343871700+00:00
updated_at: 2026-05-12T13:57:42.269798667+00:00
closed_at: 2026-05-12T13:57:42.269798586+00:00
---

# Description

## Cleanup Step Discards From Both Players (Should Be Active Player Only)

Context:
- Date: 2026-04-03
- Source code audit of turn/phase handling
- File: mtg-engine/src/game/game_loop/steps.rs:419

### Steps to Reproduce
1. Player A (active) ends turn with 8+ cards in hand
2. Player B (non-active) also has 8+ cards in hand (e.g., from Howling Mine effects)
3. Engine enters cleanup step
4. Both players are forced to discard to max hand size

### Expected Behavior (CR 514.1)
- ONLY the active player discards during the cleanup step
- The non-active player keeps their cards until their own cleanup step
- CR 514.1: "First, if the active player's hand contains more than their maximum hand size (normally seven), the active player discards enough cards to reduce their hand size to that number."

### Actual Behavior
Code at steps.rs:419:
```rust
for &player_id in &[active_player, non_active_player] {
    let hand_size = ...;
    if hand_size > max_hand_size {
        // BOTH players forced to discard
    }
}
```

The loop processes BOTH players, forcing the non-active player to also discard.

### Impact
- Players with effects that draw cards on opponent's turn (Howling Mine, Sylvan Library responses, etc.) could be incorrectly forced to discard early
- Affects old school decks with Howling Mine (Turbo Stasis has 4x Howling Mine)
- In competitive play, hand size management is critical -- wrong timing of discard is a significant rules violation

### Fix
Change the loop to only process the active player:
```rust
let player_id = active_player;
let hand_size = ...;
if hand_size > max_hand_size {
    // discard logic
}
```

### Rules Notes
- CR 514.1: Only active player discards during cleanup
- CR 514.2: Damage on permanents is removed and "until end of turn" effects end (correctly implemented)
- CR 514.3: No player receives priority unless a triggered ability triggers during cleanup
