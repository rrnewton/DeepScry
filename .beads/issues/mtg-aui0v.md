---
title: Targeted spells offered when no valid targets exist
status: closed
priority: 2
issue_type: bug
created_at: 2026-01-02T21:28:30.803271986+00:00
updated_at: 2026-01-03T04:35:21.844154475+00:00
---

# Description

## Bug Summary

The game offers actions (spells and activated abilities) when they cannot be completed:
1. Targeted spells offered when no valid targets exist
2. Spells/abilities offered when mana cost cannot be paid

## Status: FULLY FIXED (2026-01-03_#1480)

### Fixed (Part 1 - 2026-01-03_#1473):
- Instants/sorceries with unimplemented API types (like Charm, modal spells) are no longer
  offered when we don't understand their effects
- This prevents cards like Heartless Act from being offered when we can't properly validate
  their targeting requirements

### Fixed (Part 2 - 2026-01-03_#1480):
- Waterbend abilities now check cost affordability before being offered
- Added check in push_activatable_abilities() that counts:
  - Mana in player's mana pool
  - Untapped creatures/artifacts controlled by player (excluding source card)
- If total < Waterbend amount, ability is not offered

## Fix Details

Part 1: In `push_castable_spells()`, added check:
- Instants/sorceries with empty effects are skipped (likely have unimplemented API types)
- Permanents (creatures, artifacts, enchantments, planeswalkers) still castable with empty
  effects since they enter the battlefield regardless

Part 2: In `push_activatable_abilities()`, added Waterbend cost check:
- Uses `ability.cost.get_waterbend_amount()` to detect Waterbend costs
- Counts tappable permanents (creatures/artifacts) and mana pool total
- Only offers ability if total_available >= waterbend_amount

## Verification

Both reproducers now run without errors:
```bash
## Part 1 - no longer offers Heartless Act without valid targets
timeout 90 target/release/mtg tui decks/booster_draft/avatar/ryan_avatar_draft.dck \
  --p1=random --p2=random --seed=888 --log-tail=600 2>&1 | \
  grep "Failed to pay cost"

## Part 2 - no longer offers unaffordable Waterbend abilities  
timeout 60 target/release/mtg tui decks/booster_draft/avatar/gabriel_avatar_draft.dck \
  --p1=random --p2=random --seed=999 --log-tail=600 2>&1 | \
  grep "Cannot pay Waterbend"
```
