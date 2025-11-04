---
title: 'Fancy TUI: Simple stacking with multiplier prefix (e.g., 3x Island)'
status: open
priority: 3
issue_type: task
created_at: 2025-11-04T10:05:17.722161463+00:00
updated_at: 2025-11-04T10:05:17.722161463+00:00
---

# Description

## Description

Stack multiple copies of the same card that share the same tapped state into a single visual representation with a multiplier prefix (e.g., "3x Island").

This is a simpler intermediate step before full visual stacking (mtg-a07166) that provides most of the space-saving benefits without complex diagonal rendering.

## Visual Design

- Cards with the same name AND same tapped state are grouped into one stack
- Display as a single card with title "Nx CardName" (e.g., "3x Island", "2x Grizzly Bears")
- The multiplier prefix "Nx" is colorized in **Cyan** (aquamarine/highlight color)
- 3 untapped Islands + 2 tapped Islands = 2 separate stacks:
  - "3x Island" (untapped)
  - "2x Island [TAPPED]" (tapped)

## Example

Before (5 Islands, 3 untapped, 2 tapped):
```
┌────────┐ ┌────────┐ ┌────────┐ ┌────────┐ ┌────────┐
│Island  │ │Island  │ │Island  │ │Island  │ │Island  │
│        │ │        │ │        │ │ [T]    │ │ [T]    │
└────────┘ └────────┘ └────────┘ └────────┘ └────────┘
```

After (2 stacks):
```
┌────────┐ ┌────────┐
│3x Island│ │2x Island│
│        │ │ [T]    │
└────────┘ └────────┘
```

## Data Structure

Introduce `BattlefieldEntity` to abstract what can be rendered:

```rust
#[derive(Debug, Clone)]
enum BattlefieldEntity {
    SingleCard {
        card_id: CardId,
    },
    SimpleStack {
        card_ids: Vec<CardId>,  // All cards in this stack
        card_name: String,       // Shared name
        is_tapped: bool,         // Shared tapped state
        representative_card: CardId,  // Use this for rendering details
    },
}

impl BattlefieldEntity {
    fn card_ids(&self) -> Vec<CardId> {
        match self {
            Self::SingleCard { card_id } => vec![*card_id],
            Self::SimpleStack { card_ids, .. } => card_ids.clone(),
        }
    }
    
    fn representative_card(&self) -> CardId {
        match self {
            Self::SingleCard { card_id } => *card_id,
            Self::SimpleStack { representative_card, .. } => *representative_card,
        }
    }
    
    fn count(&self) -> usize {
        match self {
            Self::SingleCard { .. } => 1,
            Self::SimpleStack { card_ids, .. } => card_ids.len(),
        }
    }
}
```

## Refactoring Steps

### Phase 1: Introduce Abstraction (No Behavior Change)

1. **Add BattlefieldEntity enum** in fancy_tui_controller.rs
   - Start with just `SingleCard` variant
   - Add helper methods: `card_ids()`, `representative_card()`, `count()`

2. **Refactor CardPosition to EntityPosition**
   ```rust
   struct EntityPosition {
       entity: BattlefieldEntity,
       area: Rect,  // Bounding box
   }
   ```
   - Update `FancyTuiState` to use `Vec<EntityPosition>` instead of `Vec<CardPosition>`
   - Update mouse hit testing to work with entities

3. **Add group_cards_into_entities() function**
   ```rust
   fn group_cards_into_entities(
       cards: &[CardId], 
       view: &GameStateView,
       enable_stacking: bool,
   ) -> Vec<BattlefieldEntity> {
       if !enable_stacking {
           // Phase 1: just wrap each card
           return cards.iter()
               .map(|&card_id| BattlefieldEntity::SingleCard { card_id })
               .collect();
       }
       
       // Phase 2: group by (name, tapped_state)
       // ... implementation here
   }
   ```

4. **Update render_card_group to use entities**
   - Call `group_cards_into_entities(cards, view, false)` (stacking disabled)
   - Iterate over entities instead of cards
   - Pass entity to rendering logic

5. **Extract calculate_entity_dimensions()**
   ```rust
   fn calculate_entity_dimensions(
       entity: &BattlefieldEntity,
       view: &GameStateView,
       base_width: u16,
       base_height: u16,
   ) -> (u16, u16) {
       match entity {
           BattlefieldEntity::SingleCard { card_id } => {
               Self::get_card_dimensions_with_size(view, *card_id, base_width, base_height)
           },
           BattlefieldEntity::SimpleStack { is_tapped, .. } => {
               // Same as single card - no diagonal offset yet
               if *is_tapped {
                   (base_height, base_width)  // Swapped for tapped
               } else {
                   (base_width, base_height)
               }
           },
       }
   }
   ```

6. **Update render_card_box to render_entity()**
   - Rename and adjust to handle entities
   - For `SingleCard`, behavior unchanged
   - For `SimpleStack`, initially same as SingleCard

7. **Validate refactoring with tests**
   - Run full test suite
   - Manually test fancy TUI
   - Ensure no behavior changes

### Phase 2: Enable Simple Stacking

8. **Implement grouping logic in group_cards_into_entities()**
   - Group cards by (card_name, is_tapped)
   - Create `SimpleStack` when count > 1
   - Create `SingleCard` when count == 1

9. **Modify render_entity() for SimpleStack**
   - Detect `SimpleStack` with count > 1
   - Prepend "Nx" to card name (e.g., "3x Island")
   - Colorize "Nx" prefix in **Cyan** color
   - Render rest of card normally

10. **Update mouse selection logic**
    - Clicking a stack selects the representative card
    - Card details pane shows the representative card
    - Future: could show "(3 copies)" in details

11. **Update choice highlighting**
    - If ANY card in a SimpleStack is in valid_choices, highlight the stack
    - This handles spells that target "any Island" - the whole stack lights up

12. **Test simple stacking thoroughly**
    - Various counts (1x, 2x, 10x, etc.)
    - Mixed tapped/untapped states
    - Different card names
    - Mouse interaction
    - Choice highlighting

## Benefits Over Current System

- **Massive space savings**: 10 Islands becomes 1 stack
- **Easier to read**: "5x Mountain" is clearer than 5 separate cards
- **Better UX**: Less scrolling/wrapping needed
- **Maintains functionality**: All interactions still work (mouse, highlighting, selection)

## Benefits Over Full Visual Stacking (mtg-a07166)

- **Much simpler to implement**: No diagonal geometry, no partial card rendering
- **Easier hit testing**: Same bounding box logic as single cards
- **Same layout algorithm**: Stacks use same dimensions as single cards
- **Lower risk**: Minimal changes to rendering pipeline

## Relationship to mtg-a07166 (Full Visual Stacking)

This issue is a **prerequisite** for mtg-a07166. The abstractions and refactorings done here (BattlefieldEntity, EntityPosition, entity-based rendering) will make implementing full visual stacking much easier later.

The upgrade path from simple → visual stacking:
1. Add `VisualStack` variant to `BattlefieldEntity` enum
2. Update `calculate_entity_dimensions()` to account for diagonal offsets
3. Implement `render_visual_stack()` with partial card rendering
4. Everything else (grouping, hit testing, selection) already works!

## Implementation Phases

- ✅ Phase 1: Refactor to use BattlefieldEntity abstraction (steps 1-7)
- ⏸ Phase 2: Enable simple stacking with multiplier prefix (steps 8-12)

Blocks: mtg-a07166
Part of: mtg-dba689
