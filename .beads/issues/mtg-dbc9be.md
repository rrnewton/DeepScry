---
title: TurnStructure.active_player_idx not updated on turn transitions
status: open
priority: 3
issue_type: task
labels:
- single-card
created_at: 2026-04-03T21:20:55.428313845+00:00
updated_at: 2026-04-03T21:20:55.428313845+00:00
---

# Description

## active_player_idx stale after next_turn()

Context:
- Date: 2026-04-03
- Source code audit of turn transition logic
- Found during extra turn / temporal effects playtesting audit

### Steps to Reproduce
1. Start a 2-player game (player 0 goes first)
2. Player 0 completes turn 1
3. advance_step() in state.rs:1714 calls self.turn.next_turn(next_player)
4. next_turn() in phase.rs:289-295 sets active_player but NOT active_player_idx
5. active_player_idx remains 0 (starting player) for the rest of the game

### Expected Behavior
- active_player_idx should track the index of the active player in the players Vec
- It should update every time active_player changes

### Actual Behavior
- active_player_idx is set in constructors (new/new_with_idx) and by puzzle loader
- next_turn() updates active_player but NOT active_player_idx
- The undo system (undo.rs:491) correctly restores it, but the forward path does not set it

### Impact
- WASM frontend (wasm/mod.rs:601) uses active_player_idx for get_state_json()
- After turn 1, the frontend reports the wrong active player
- The field is essentially dead code in normal game flow (only correct on turn 1 and after undo)

### Fix
In phase.rs next_turn(), add:
```rust
pub fn next_turn(&mut self, next_player: crate::core::PlayerId) {
    self.turn_number += 1;
    self.current_step = Step::Untap;
    self.active_player = next_player;
    // BUG FIX: also need to update active_player_idx
    // But next_turn doesn't have access to the players Vec...
    self.priority_player = None;
    self.consecutive_passes = 0;
}
```

The fix requires either:
1. Passing the new player index to next_turn(), or
2. Having the caller (state.rs advance_step) update active_player_idx after calling next_turn()

### Rules Notes
- Not a rules violation per se, but affects state reporting correctness
- CR 500.1: active player is the player whose turn it is
