---
title: 'Fancy TUI: Proportionate card rectangles and 2D battlefield layout'
status: open
priority: 3
issue_type: task
created_at: 2025-11-03T16:35:32.051260815+00:00
updated_at: 2025-11-03T16:35:32.051260815+00:00
---

# Description

Part of: mtg-dba689

Display cards with aspect ratio close to real MTG cards (3.5:2.5 = 1.4:1) and arrange battlefield in 2D grid.

## Current implementation

Cards are displayed as simple boxes in vertical groups:
```
Creatures:
┌─────────────────┐
│ Grizzly Bears   │
│                 │
└─────────────────┘
```

## Target implementation

Cards displayed at ~1.4:1 ratio (accounting for terminal character aspect ~2:1), arranged in grid:

```
Creatures:
┌────────┐ ┌────────┐ ┌────────┐
│Grizzly │ │Elvish  │ │Savannah│
│Bears   │ │Warrior │ │Lions   │
│  2/2   │ │  1/1   │ │  2/1   │
└────────┘ └────────┘ └────────┘
```

For terminal display, this means cards should be approximately:
- Width: ~10 characters
- Height: ~7 lines (to approximate 1.4:1 accounting for terminal font aspect ratio)

## Implementation challenges

- Calculate optimal card dimensions based on available space
- Implement 2D layout algorithm (rows and columns)
- Handle varying numbers of cards (wrap to multiple rows)
- Truncate card names if needed
- Show P/T, tap status, etc. in compact form
- Consider horizontal scrolling if too many cards

## Layout algorithm

1. Calculate available area (battlefield pane dimensions)
2. Determine card dimensions (fixed or adaptive)
3. Calculate max cards per row: `width / (card_width + spacing)`
4. Arrange cards in rows, within each card type group
5. Render each card at calculated position

## Files to modify

- `src/game/fancy_tui_controller.rs`:
  - `render_card_group`: Change from vertical list to 2D grid
  - `render_card_box`: Adjust dimensions to match new aspect ratio
  - May need new helper: `calculate_card_layout`

## Dependencies

Should be done before: mtg-TBD (card border colors) - easier to add colors to well-formatted cards
