---
title: 'Fancy TUI: Populate Card Details pane on selection'
status: closed
priority: 3
issue_type: task
created_at: 2025-11-03T18:06:09.550540390+00:00
updated_at: 2025-11-03T18:56:50.132012338+00:00
closed_at: 2025-11-03T18:56:50.132012207+00:00
---

# Description

Part of: mtg-dba689

Populate the Card Details pane when a card is selected via mouse click or keyboard navigation.

## Current behavior

Card Details pane shows "(No card selected)" at all times.

## Target behavior

When a card is clicked or selected:
- Card Details pane displays the card's information
- Name (highlighted in yellow/bold)
- Type line (e.g., "Creature - Human Soldier")
- Mana cost
- Power/Toughness (for creatures)
- Card text/abilities

## Implementation

The infrastructure is already there in `draw_card_details`:
- `self.state.selected_card_id` holds the selected card
- Card info is already displayed if `selected_card_id` is Some

What's needed:
- Set `selected_card_id` when user clicks a card (requires mtg-1a7bae mouse support)
- Alternative: Set `selected_card_id` to highlighted choice card during choices
- Clear `selected_card_id` when appropriate (e.g., choice complete)

## Files to modify

- `src/game/fancy_tui_controller.rs`:
  - Mouse click handler (when implemented) sets `selected_card_id`
  - Choice methods can set `selected_card_id` to current highlighted card
  - `draw_card_details` already works correctly

## Dependencies

- Fully functional with: mtg-1a7bae (mouse support)
- Partially functional now: Can populate during choices based on highlighted option

## Testing

- Start game with `--p1=fancy`
- During a choice (e.g., casting a spell), navigate choices with arrow keys
- Verify Card Details updates to show the card being considered
- (After mouse support) Click a card on battlefield
- Verify Card Details shows that card's information
