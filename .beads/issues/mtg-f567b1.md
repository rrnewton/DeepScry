---
title: 'Reorganize fancy TUI layout: move Stack and Actions'
status: closed
priority: 1
issue_type: task
created_at: 2025-11-03T20:20:04.318818209+00:00
updated_at: 2025-11-03T20:41:36.688872811+00:00
closed_at: 2025-11-03T20:41:36.688872601+00:00
---

# Description

## Goal

Reorganize the fancy TUI layout to better utilize screen space and improve information hierarchy.

## Current Layout

**Left Column (25%):**
- Stack|Combat|Log tabs (60%)
- Prompt|Dock tabs (40%)

**Center Column (50%):**
- Opponent info + battlefield
- Player info + battlefield

**Right Column (25%):**
- Card Details (50%)
- Hand (50%)

## Target Layout

**Left Column (25%):**
- Combat|Log tabs (leave Log as active-by-default)
- Move Stack OUT of these tabs

**Center Column (50%):**
- Unchanged (battlefields)

**Right Column (25%):**
- Card Details (33%)
- Hand (33%)
- Stack (33%) - moved from left column

## Changes Required

1. **Move Stack pane:**
   - Remove Stack from left column tabs
   - Add Stack as new pane in right column below Hand
   - Keep only Combat|Log tabs in upper left
   - Set Log as active-by-default tab

2. **Remove Dock tab:**
   - Dock was for deck management, not implementing it
   - Consolidate Prompt as the only bottom-left pane
   - Remove Prompt|Dock tabs, just show Prompt directly

3. **Adjust vertical splits in right column:**
   - Card Details: 33% (was 50%)
   - Hand: 33% (was 50%)
   - Stack: 33% (new)

## Rationale

- Stack is important game state, deserves its own permanent pane
- Putting Stack near Hand makes sense (both are player-specific views)
- Removes clutter from left column tabs
- Dock tab was never going to be implemented

## Files

- : Main layout changes

## Testing

- Start game with 
- Verify Stack appears in right column below Hand
- Verify left column shows Combat|Log tabs only
- Verify Prompt pane shows directly (no tabs)
- Cast spells and verify Stack pane updates correctly
