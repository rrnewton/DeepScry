---
title: Targeted spells offered when no valid targets exist
status: open
priority: 2
issue_type: bug
created_at: 2026-01-02T21:28:30.803271986+00:00
updated_at: 2026-01-02T21:29:32.505767248+00:00
---

# Description

## Bug Summary

The game offers actions (spells and activated abilities) when they cannot be completed:
1. Targeted spells offered when no valid targets exist
2. Spells/abilities offered when mana cost cannot be paid

## Reproducer 1: Targeted Spells

```bash
timeout 90 target/release/mtg tui decks/booster_draft/avatar/ryan_avatar_draft.dck \
  --p1=random --p2=random --seed=888 --log-tail=600 2>&1 | \
  grep -B30 "cast Heartless Act" | grep -E "(Player1:|Player2:|Battlefield)"
```

Heartless Act was castable when no creatures existed on the battlefield.

## Reproducer 2: Unaffordable Costs

```bash
timeout 90 target/release/mtg tui decks/booster_draft/avatar/gabriel_avatar_draft.dck \
  --p1=random --p2=random --seed=999 --log-tail=600 2>&1 | \
  grep "Failed to pay cost"
```

Shows many "Cannot pay Waterbend 5: only X available" errors - Foggy Swamp Vinebender's ability is being offered when it can't be paid.

## Expected Behavior

1. Spells that require targets should only be offered when at least one valid target exists
2. Spells and activated abilities should only be offered when their costs can be paid

## Affected Cards

- Heartless Act (targets creatures)
- Fatal Fissure (likely)
- Lightning Strike (when targeting creatures)
- Foggy Swamp Vinebender (Waterbend 5 ability)
- Other Waterbend abilities

## Root Cause (Suspected)

The action generation code doesn't validate:
1. Target availability before offering targeted spells
2. Cost affordability before offering spells/abilities

## Notes

Found during random/random playtesting. The heuristic AI avoids these by making smarter choices.
