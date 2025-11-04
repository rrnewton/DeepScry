---
title: 'Fancy TUI: Enhanced choice highlighting in gameplay'
status: open
priority: 3
issue_type: task
created_at: 2025-11-03T16:37:22.509067115+00:00
updated_at: 2025-11-03T16:37:22.509067115+00:00
---

# Description

Part of: mtg-dba689

Improve how choices are presented and highlighted during gameplay to make the fancy TUI distinct from GUI versions.

## Current implementation

Choices are shown in the Prompt pane with:
- Numbered options: [0] Pass, [1] Cast X, etc.
- Yellow background for selected choice
- Navigate with arrow keys, select with Enter

## Enhancements

### 1. Highlight valid cards in Hand/Battlefield

When asked to play a spell or activate an ability:
- Highlight valid cards in Hand pane (cards that can be played)
- Dim cards that can't be played
- Similar for Battlefield (permanents with activated abilities)

Example: During main phase with 3 mana available:
- Cards costing ≤3 mana: Normal/bright display
- Cards costing >3 mana: Dimmed (DarkGray)
- Lands already played this turn: Dimmed

### 2. Visual connection between Prompt and cards

When navigating choices with arrow keys:
- Highlight the corresponding card in Hand/Battlefield
- Update Card Details to show the card being considered

Example: Prompt shows "Cast Shock", highlighting moves to Shock in Hand, Card Details shows Shock.

### 3. Combat phase highlighting

**Declare Attackers:**
- Highlight available attackers on Your Battlefield
- Already-declared attackers shown with different style (e.g., red border)
- Tapped/summoning sick creatures dimmed

**Declare Blockers:**
- Highlight available blockers on Your Battlefield
- Highlight attackers on Opponent Battlefield (potential block targets)
- Show blocking assignments visually (lines/arrows if feasible in terminal)

### 4. Target selection highlighting

When choosing a target:
- Highlight valid targets on Battlefield
- Dim invalid targets
- Show ownership (ties into mtg-7bbb00)

## Implementation approach

### State tracking
```rust
struct FancyTuiState {
    // ... existing ...
    valid_choices: Vec<CardId>,  // Cards that can currently be chosen
    choice_context: ChoiceContext,  // What kind of choice is being made
}

enum ChoiceContext {
    PlayingSpell,
    ActivatingAbility,
    DeclareAttackers,
    DeclareBlockers,
    TargetSelection,
    None,
}
```

### Rendering changes

In `draw_hand` and `draw_battlefield`:
```rust
let style = if self.state.valid_choices.contains(&card_id) {
    Style::default().fg(Color::White)  // Bright
} else if self.state.choice_context != ChoiceContext::None {
    Style::default().fg(Color::DarkGray)  // Dimmed
} else {
    Style::default().fg(Color::White)  // Normal
};
```

### Choice methods

Update choice methods to populate `valid_choices` and `choice_context`:
- `choose_spell_ability_to_play`: Extract card IDs from available abilities
- `choose_attackers`: Set available_creatures as valid_choices
- `choose_blockers`: Set blockers + attackers as valid_choices
- `choose_targets`: Set valid_targets as valid_choices

## Files to modify

- `src/game/fancy_tui_controller.rs`:
  - Add `ChoiceContext` enum
  - Extend `FancyTuiState` with choice tracking
  - Update all choice methods to set context
  - Modify rendering to respect valid_choices

## Dependencies

- Works better with: mtg-b3f1fe (pane focus) - navigate highlighted cards
- Works better with: mtg-fa9417 (2D battlefield) - easier to see highlighted cards
- Related to: mtg-7bbb00 (ownership/IDs) - both improve choice clarity

## Testing

- Start game, advance to main phase
- Verify playable cards in hand are highlighted
- Cast a targeting spell
- Verify valid targets are highlighted on battlefield
- Declare attackers
- Verify available attackers are highlighted
