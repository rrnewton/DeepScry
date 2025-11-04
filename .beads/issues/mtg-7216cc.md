---
title: 'game_loop: Replace println/eprintln with logger calls'
status: open
priority: 3
issue_type: task
created_at: 2025-11-04T11:24:03.711987394+00:00
updated_at: 2025-11-04T11:24:03.711987394+00:00
---

# Description

## Description

The game_loop.rs file uses println!/eprintln! for most game actions instead of using the game logger. This causes issues for the fancy TUI where the log pane is missing entries.

## Problems

1. Damage logging goes to stdout instead of logger
2. Combat logs go to stdout instead of logger  
3. Player actions go to stdout instead of logger
4. Spell resolutions go to stdout instead of logger
5. This makes the fancy TUI log pane very sparse

## Solution

Replace all println!/eprintln! calls in game_loop.rs with logger calls:
- self.game.logger().log(VerbosityLevel::Normal, message)
- For errors: self.game.logger().log(VerbosityLevel::Minimal, message)

## Specific Areas

Lines with println!/eprintln! in game_loop.rs:
- Line 1208: Mana tapping (actions.rs)
- Lines 1097, 1106, 1112: Damage logging
- Lines 1787, 1797: Combat damage logging
- Lines 1542, 1657: Attack/block declarations
- Lines 2227, 2234, 2266, 2273: Spell/land play logging
- Lines 2372: Ability activation logging

## Improvements Needed

1. Centralize damage logging to include new life totals
2. Format combat logs: "Combat: CardA (id) (X damage) ↔ CardB (id) (Y damage)"
3. Ensure all player actions are logged (P1 and P2)
4. Add logging for attacks, blocks with proper formatting

Part of: mtg-dba689
