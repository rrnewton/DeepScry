---
title: Targeted spells offered when no valid targets exist
status: open
priority: 2
issue_type: bug
created_at: 2026-01-02T21:28:30.803271986+00:00
updated_at: 2026-01-03T02:30:20.020416522+00:00
---

# Description

## Bug Summary

The game offers actions (spells and activated abilities) when they cannot be completed:
1. Targeted spells offered when no valid targets exist
2. Spells/abilities offered when mana cost cannot be paid

## Status: PARTIALLY FIXED (2026-01-03_#1473)

### Fixed:
- Instants/sorceries with unimplemented API types (like Charm, modal spells) are no longer
  offered when we don't understand their effects
- This prevents cards like Heartless Act from being offered when we can't properly validate
  their targeting requirements

### Still Open:
- The second issue (Waterbend abilities offered when cost can't be paid) needs investigation
  - This may be an issue with alternative cost handling in push_activatable_abilities()
  - Needs separate fix

## Fix Details

In `push_castable_spells()`, added check:
- Instants/sorceries with empty effects are skipped (likely have unimplemented API types)
- Permanents (creatures, artifacts, enchantments, planeswalkers) still castable with empty
  effects since they enter the battlefield regardless

## Reproducer 1: Targeted Spells (NOW FIXED)

```bash
timeout 90 target/release/mtg tui decks/booster_draft/avatar/ryan_avatar_draft.dck \
  --p1=random --p2=random --seed=888 --log-tail=600 2>&1 | \
  grep -B30 "cast Heartless Act" | grep -E "(Player1:|Player2:|Battlefield)"
```

Heartless Act was castable when no creatures existed on the battlefield.

## Reproducer 2: Unaffordable Costs (STILL OPEN)

```bash
timeout 90 target/release/mtg tui decks/booster_draft/avatar/gabriel_avatar_draft.dck \
  --p1=random --p2=random --seed=999 --log-tail=600 2>&1 | \
  grep "Failed to pay cost"
```

Shows many "Cannot pay Waterbend 5: only X available" errors - Foggy Swamp Vinebender's ability is being offered when it can't be paid.
