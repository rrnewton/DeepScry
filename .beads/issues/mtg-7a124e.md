---
title: Fix double-printing of menu prompts in TUI
status: closed
priority: 3
issue_type: task
created_at: 2025-11-03T19:50:59.133988638+00:00
updated_at: 2025-11-03T20:13:47.640294641+00:00
closed_at: 2025-11-03T20:13:47.640294591+00:00
---

# Description

## Description

## Problem

When using mtg tui with fixed or random controllers, menu prompts were being printed twice.

Example of the bug:
```
Bob available actions:
  [0] Play land: Forest
  [1] Play land: Mountain

Bob available actions:
  [0] Play land: Forest
  [1] Play land: Mountain
  Bob chose 1 - Play land: Mountain
```

## Root Cause

The menu was being printed in TWO locations:
1. **game_loop.rs:2133** - The game loop was printing it centrally
2. **Controllers** - Both random_controller.rs:78 and heuristic_controller.rs:2383 were ALSO printing it

This caused double-printing because both the game loop AND the individual controllers were calling `format_choice_menu()`.

## Solution Implemented

Removed duplicate menu printing from controllers:
- Removed lines 77-79 from random_controller.rs
- Removed lines 2381-2384 from heuristic_controller.rs
- Removed unused `use crate::game::format_choice_menu` imports from both files

Now the menu is only printed once by the game loop, maintaining a single source of truth for menu display.

## Files Modified

- `src/game/random_controller.rs` - Removed duplicate print statement
- `src/game/heuristic_controller.rs` - Removed duplicate print statement

## Testing

Verified fix with:
```bash
cargo run --release --bin mtg -- tui decks/julian_spiderman_draft.dck decks/ryan_spiderman_draft.dck --p1=fixed --p1-fixed-inputs="0" --p2=random --stop-on-choice=2 --seed=42
```

Output now shows menu exactly once per decision point.
