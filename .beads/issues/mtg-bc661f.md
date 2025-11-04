---
title: 'Fancy TUI: Card border colors reflecting mana colors'
status: closed
priority: 3
issue_type: task
created_at: 2025-11-03T16:35:44.679531056+00:00
updated_at: 2025-11-03T17:29:16.636994588+00:00
closed_at: 2025-11-03T17:29:16.636994448+00:00
---

# Description

Part of: mtg-dba689

Make card borders reflect the card's color identity using terminal colors.

## Color mapping

- Red (R): `Color::Red`
- Green (G): `Color::Green`
- Blue (U): `Color::Blue`
- White (W): `Color::White`
- Black (B): `Color::DarkGray` (darker grey)
- Colorless: `Color::Gray`
- Multicolor: Cycle colors or use `Color::Yellow`/`Color::Magenta`

## Implementation

In `render_card_box`:
```rust
let border_color = match card.colors.len() {
    0 => Color::Gray,  // Colorless
    1 => match card.colors[0] {
        'R' => Color::Red,
        'G' => Color::Green,
        'U' => Color::Blue,
        'W' => Color::White,
        'B' => Color::DarkGray,
        _ => Color::Gray,
    },
    _ => Color::Yellow,  // Multicolor
};

let block = Block::default()
    .borders(Borders::ALL)
    .border_style(Style::default().fg(border_color))
    .style(base_style);
```

## Considerations

- Card color vs. mana cost color (use card.colors field)
- Lands: Usually colorless but produce colored mana - decide on representation
- Tapped cards: Maybe dimmer version of color?
- Works with: mtg-fa9417 (after 2D layout is implemented)

## Files to modify

- `src/game/fancy_tui_controller.rs`: `render_card_box` method
