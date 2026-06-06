---
title: 'Bug: Count$ValidGraveyard not implemented in CountExpression parser'
status: open
priority: 2
issue_type: task
created_at: 2026-06-06T04:29:14.154914584+00:00
updated_at: 2026-06-06T04:29:14.154914584+00:00
---

# Description

Missing `Count$ValidGraveyard` support in CountExpression::parse() in mtg-engine/src/core/effects.rs.

Root cause: CountExpression::parse_internal() handles Count$ValidHand, Count$Valid (permanents), Count$YouDrewThisTurn, Count$YouCastThisTurn, Count$Compare, and Count$xPaid — but NOT Count$ValidGraveyard. When a SVar has `Count$ValidGraveyard Lesson.YouOwn/Plus.2`, it returns Fixed(0).

Affected cards (examples from 2025 WC Izzet Lessons):
- Combustion Technique: SVar:X:Count$ValidGraveyard Lesson.YouOwn/Plus.2 → always deals 0 damage (should be 2 + lessons)
- Accumulate Wisdom: SVar:Y:Count$ValidGraveyard Lesson.YouOwn → Compare expression Y>=3 always false → always puts 1 card instead of 3
- Gran-Gran cost reduction: S:Mode$ ReduceCost | IsPresent$ Lesson.YouOwn | PresentZone$ Graveyard | PresentCompare$ GE3 (different mechanism but same theme)

Fix needed: Add Count$ValidGraveyard parsing to CountExpression::parse_internal() similar to Count$ValidHand. It should count cards in the graveyard matching the given selector (e.g., Lesson.YouOwn = Lesson cards owned by the current player).

The Plus.N modifier suffix (Count$ValidGraveyard Lesson.YouOwn/Plus.2) must also be handled — add a CountModifier to the expression.

Reproducer:
```sh
./target/release/mtg tui --p1 zero --p2 zero --p1-draw "Combustion Technique;Island;Island;Mountain;Island;Island;Island" --p2-draw "Island;Island;Island;Island;Island;Island;Island" --seed 42 --verbosity 3 decks/championship/2025/01_manfield_izzet_lessons.dck decks/championship/2025/01_manfield_izzet_lessons.dck 2>&1 | grep -E "Combustion|deals|damage"
```

Expected (with Lessons in graveyard): "Combustion Technique deals 4 damage to ..."
Actual: "Combustion Technique deals 0 damage to ..."

Findings (2026-06-05_#3008(50175e06)):
- Count$ValidGraveyard not in CountExpression::parse_internal()
- Falls through to Fixed(0) default
- Affects Combustion Technique, Accumulate Wisdom, and any card counting graveyard cards

CARD STATUS: BROKEN (root-cause bug, not card-specific)
