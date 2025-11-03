---
title: Fix double-printing of menu prompts in TUI
status: open
priority: 3
issue_type: task
created_at: 2025-11-03T19:50:59.133988638+00:00
updated_at: 2025-11-03T19:50:59.133988638+00:00
---

# Description

## Problem

When using mtg tui with --p1=fixed or --p2=fixed, menu prompts are being printed twice.

## Expected Behavior

Menu should only be printed once per decision point.

## Investigation Needed

- Check if RichInputController is somehow triggering double display
- Review game_loop.rs menu printing logic
- May be related to logger/stdout interaction

## Priority

Medium - cosmetic issue but affects user experience and log readability.
