---
title: TUI shows wrong player names and incorrect available actions
status: closed
priority: 2
issue_type: bug
created_at: 2025-10-27T01:59:56+00:00
updated_at: 2025-11-04T12:50:32.690460730+00:00
---

# Description

## Problems

1. ✅ **Player naming**: Shows "Player 0" instead of actual player names
   - **FIXED** in commit 17218563: Changed fallback formatting from `{:?}` to use `as_u32() + 1`
   - Now shows "Player 1", "Player 2" instead of "Player(0)", "Player(1)"
   - Actual player names (Player1, Player2) continue to work as expected
   - **UPDATE 2025-11-04**: Default names changed from Alice/Bob to Player1/Player2 (commit b865847)

2. ✅ **"Your Turn" message**: Code review shows proper "Priority {player_name}" not "Your Turn"
   - Current code at interactive_controller.rs line 236-242 is correct
   - Shows priority during priority rounds, not "Your Turn"

3. ✅ **Stack checking**: Code properly checks stack before offering sorcery-speed actions
   - `get_available_spell_abilities` checks `stack_is_empty` (line 2244)
   - Land plays only added when `stack_is_empty` (lines 2248-2260)
   - `get_castable_spells` checks `stack_is_empty` (line 2117)
   - Sorceries require `stack_is_empty` (line 2133)

## Status

**FIXED** - All issues addressed:
- Player name formatting works correctly (Player 1, Player 2 or custom names)
- Priority messages are correct (not "Your Turn")
- Stack checking prevents sorcery-speed actions when inappropriate

Closing this issue as all items are verified correct via code review.
