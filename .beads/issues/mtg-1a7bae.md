---
title: 'Fancy TUI: Mouse support for card selection'
status: open
priority: 3
issue_type: task
created_at: 2025-11-03T16:36:35.032190062+00:00
updated_at: 2025-11-03T16:36:35.032190062+00:00
---

# Description

Part of: mtg-dba689

Add mouse support for selecting cards and viewing details.

## Features

### Click on card
- Click any card in Hand or Battlefield -> select it and show in Card Details pane
- Clicked card is highlighted (bold text or different border)

### Click in panes
- Optionally: Click in a pane to focus it (complementing keyboard shortcuts)

### Visual feedback
- Highlighted/selected card shown with bold text
- Or use different border style for selected card

## Implementation

### Enable mouse events

In `setup_terminal`:
```rust
use crossterm::event::{EnableMouseCapture, DisableMouseCapture};

execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
```

In `restore_terminal`:
```rust
execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
```

### Handle mouse events

In input event loop:
```rust
if let Event::Mouse(mouse_event) = event::read()? {
    match mouse_event.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            let (x, y) = (mouse_event.column, mouse_event.row);
            // Determine which pane was clicked
            // Determine which card (if any) was clicked
            // Update selected_card_id
            return Ok(InputAction::Continue); // Redraw with new selection
        }
        _ => {}
    }
}
```

### Hit testing

Need to track the screen positions of rendered cards:
```rust
struct CardPosition {
    card_id: CardId,
    area: Rect,
}

// Store during rendering, use during click handling
```

## Challenges

- Tracking card positions during rendering
- Hit testing: which card is at (x, y)?
- Works best after 2D battlefield layout (mtg-fa9417)

## Dependencies

- Requires: mtg-b3f1fe (pane focus) - mouse clicks should update focus state
- Works better with: mtg-fa9417 (2D battlefield) - easier to click individual cards

## Files to modify

- `src/game/fancy_tui_controller.rs`:
  - Enable/disable mouse capture in setup/restore
  - Add mouse event handling
  - Track card positions during rendering
  - Implement hit testing
