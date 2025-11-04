---
title: 'Fancy TUI: Enhanced choice highlighting in gameplay'
status: closed
priority: 3
issue_type: task
created_at: 2025-11-03T16:37:22.509067115+00:00
updated_at: 2025-11-04T01:22:49.019675510+00:00
---

# Description

Part of: mtg-dba689

Improve how choices are presented and highlighted during gameplay to make the fancy TUI distinct from GUI versions.

## Status

✅ **COMPLETED** (2025-11-04)

## Implementation

### Core Changes

Added `ChoiceContext` enum to track what kind of choice is being made:
- `PlayingSpell`: When choosing spells/abilities to play  
- `DeclareAttackers`: When choosing creatures to attack with
- `DeclareBlockers`: When choosing blockers and attackers to block
- `TargetSelection`: When choosing targets for spells/abilities
- `None`: No active choice

Added `valid_choices: Vec<CardId>` to `FancyTuiState` to track which cards are valid in current context.

### Visual Highlighting

Modified `render_card_box()` to apply highlighting based on choice context:
- **Valid choices**: Displayed in bright white (normal appearance)
- **Invalid choices**: Dimmed to dark gray when a choice context is active
- **No context**: Normal display based on tapped state

### Choice Method Updates

Updated all choice methods to set and clear context:

1. **`choose_spell_ability_to_play()`**:
   - Sets context to `PlayingSpell`
   - Extracts card IDs from available abilities
   - Highlights playable cards in Hand
   - Clears context after choice

2. **`choose_targets()`**:
   - Sets context to `TargetSelection`
   - Shows valid_targets as bright cards
   - Dims invalid targets on battlefield
   - Clears context after choice

3. **`choose_attackers()`**:
   - Sets context to `DeclareAttackers`
   - Highlights available attackers
   - Dims tapped/summoning sick creatures
   - Clears context after choice

4. **`choose_blockers()`**:
   - Sets context to `DeclareBlockers`
   - Highlights both available blockers (Your Battlefield) and attackers (Opponent Battlefield)
   - Shows which cards are involved in combat
   - Clears context after choice

## Files Modified

- `src/game/fancy_tui_controller.rs`:
  - Added `ChoiceContext` enum (lines 70-83)
  - Added `valid_choices` and `choice_context` fields to `FancyTuiState`
  - Modified `render_card_box()` to apply highlighting (lines 1005-1023)
  - Updated `choose_spell_ability_to_play()` (lines 1820-1871)
  - Updated `choose_targets()` (lines 1873-1934)
  - Updated `choose_attackers()` (lines 1967-2005)
  - Updated `choose_blockers()` (lines 2007-2064)

## Test Results

All tests passing:
- 363 unit tests (mtg_forge_rs)
- 42 AI heuristic tests
- 6 shell script tests (including controlled_draw_e2e)
- 8 TUI e2e tests  
- 3 undo e2e tests
Total: 405 tests passed

## User Experience Improvements

### Before
- All cards displayed equally during choices
- Hard to tell which cards were playable
- Users had to mentally track mana costs and availability

### After
- **Main Phase**: Playable cards in hand bright, unplayable cards dimmed
- **Combat - Attackers**: Available attackers bright, tapped/sick creatures dimmed
- **Combat - Blockers**: Both blockers and attackers highlighted
- **Targeting**: Valid targets bright, invalid targets dimmed
- Instant visual feedback shows what's possible

## Future Enhancements (Not In Scope)

- Visual connection: Auto-focus/highlight card when navigating choices with arrow keys
- Combat assignments: Show blocking assignments visually
- Border highlighting: Different border colors for valid choices vs selected
- Hand highlighting: Extend to Hand pane list items
