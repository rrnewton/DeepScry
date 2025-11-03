---
title: 'Fancy TUI: Show ownership and IDs in target choices'
status: closed
priority: 3
issue_type: task
created_at: 2025-11-03T16:36:55.243714875+00:00
updated_at: 2025-11-03T17:24:26.611005567+00:00
---

# Description

Part of: mtg-dba689

When choosing targets, clearly indicate ownership and show card IDs when there are duplicates.

## Current issue

When prompted to target a creature, display shows:
```
[0] No target
[1] Grizzly Bears
[2] Grizzly Bears
```

It's unclear:
1. Which player controls each creature
2. Which is which when there are duplicates

## Target display

### Ownership indicators
```
[0] No target
[1] Grizzly Bears (yours)
[2] Grizzly Bears (theirs)
```

Or with symbols:
```
[0] No target
[1] Grizzly Bears ⬆️  (yours)
[2] Grizzly Bears ⬇️  (theirs)
```

### Card IDs for duplicates

When multiple copies of same card:
```
[0] No target
[1] Grizzly Bears #42 (yours)
[2] Grizzly Bears #73 (theirs)
[3] Grizzly Bears #85 (theirs)
```

Show ID only when there are multiple copies of a card in the choice list.

## Implementation

In `choose_targets` method:
```rust
// Group by card name to detect duplicates
let mut name_counts: HashMap<String, usize> = HashMap::new();
for &card_id in valid_targets {
    let name = view.card_name(card_id).unwrap_or_default();
    *name_counts.entry(name.clone()).or_insert(0) += 1;
}

let choices: Vec<String> = std::iter::once("No target".to_string())
    .chain(valid_targets.iter().map(|&card_id| {
        let name = view.card_name(card_id).unwrap_or_default();
        let controller = view.get_card(card_id).map(|c| c.controller);
        let ownership = if controller == Some(self.player_id) {
            "(yours)"
        } else {
            "(theirs)"
        };
        
        // Show ID only if duplicates exist
        let id_part = if *name_counts.get(&name).unwrap_or(&0) > 1 {
            format!(" #{}", card_id.0) // Assuming CardId has accessible inner value
        } else {
            String::new()
        };
        
        let tapped = if view.is_tapped(card_id) { " [T]" } else { "" };
        format!("{}{}{} {}", name, id_part, tapped, ownership)
    }))
    .collect();
```

## Also applies to

- `choose_attackers`: Show which creatures are available
- `choose_blockers`: Show which creatures can block which attackers (with ownership)
- `choose_cards_to_discard`: Not needed (always your cards)
- `choose_spell_ability_to_play`: May show ownership if abilities from opponent's cards

## Files to modify

- `src/game/fancy_tui_controller.rs`:
  - `choose_targets`
  - `choose_blockers`
  - Possibly `choose_attackers` and `choose_spell_ability_to_play`

## Testing

- Start a game where both players have creatures
- Cast a spell targeting a creature
- Verify ownership is shown: "Grizzly Bears (yours)" vs "Grizzly Bears (theirs)"
- Have duplicate creatures on battlefield
- Verify IDs are shown only for duplicates
