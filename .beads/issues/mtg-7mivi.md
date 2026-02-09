---
title: RemoteController missing choose_blocker_for_lethal_damage implementation
status: open
priority: 2
issue_type: task
labels:
- bug
created_at: 2026-02-08T22:50:43.893827379+00:00
updated_at: 2026-02-09T17:04:40.219317062+00:00
---

# Description

## ROOT CAUSE IDENTIFIED

### Summary
`RemoteController` does not implement `choose_blocker_for_lethal_damage()`, causing it to use the default implementation that just picks the first blocker. Meanwhile, `HeuristicController` evaluates creatures and picks the most valuable one to kill.

**CardIds ARE synchronized** - the issue is that the server never asks the client for damage assignment preferences.

## Evidence

### LOCAL log (Turn 13):
```
<Choice> assign lethal damage to The Boulder, Ready to Rumble (72) first (eval=220, power=5 for 34)
[GAMELOG Turn13 CD] Mongoose Lizard (34) dies from combat damage
[GAMELOG Turn13 CD] The Boulder, Ready to Rumble (72) dies from combat damage
```

### SERVER log (Turn 13):
```
[GAMELOG Turn13 CD] Mongoose Lizard (34) dies from combat damage  
[GAMELOG Turn13 CD] White Lotus Reinforcements (73) dies from combat damage
```

Note: NO `<Choice>` line on server - it used the default implementation.

## Code Analysis

### The Problem
In `mtg-engine/src/network/remote_controller.rs`:
- Has 0 occurrences of `choose_blocker_for_lethal_damage`
- Has 0 occurrences of `choose_blocker_for_remaining_damage`
- Falls back to DEFAULT implementation in `controller.rs:1326-1328`:
  ```rust
  // Default: use the first killable blocker (fallback behavior)
  if let Some((blocker_id, _)) = killable_blockers.first() {
      ChoiceResult::Ok(*blocker_id)
  ```

### Heuristic Controller (works correctly)
In `mtg-engine/src/game/heuristic_controller.rs:4459-4504`:
```rust
fn choose_blocker_for_lethal_damage(...) -> ChoiceResult<CardId> {
    // Evaluate each killable blocker and pick the most valuable one to kill first
    for &(blocker_id, lethal_damage) in killable_blockers {
        let eval = self.evaluate_creature(view, blocker_id);
        if eval > best_eval {
            best_eval = eval;
            best_blocker = blocker_id;
        }
    }
}
```

## The Fix

`RemoteController` needs to implement `choose_blocker_for_lethal_damage` to:
1. Send a network request to the client asking for the damage order choice
2. Wait for the client's heuristic controller to make the decision
3. Return the client's choice

Same for `choose_blocker_for_remaining_damage`.

## Files Affected
- `mtg-engine/src/network/remote_controller.rs` - needs implementations
- `mtg-engine/src/network/protocol.rs` - may need new message types
- `mtg-engine/src/network/local_controller.rs` - client-side handling

## Reproduction
```bash
python3 bug_finding/network_fuzz_test.py --local-equivalence --configs 1 --parallel 1
## Or: ./tests/network_vs_local_equivalence_e2e.sh 3 heuristic heuristic
```

100% reproducible with seeds 1, 3, 5 using heuristic controllers.
