---
title: 'Card Compatibility: Firebending Lesson'
status: open
priority: 3
issue_type: task
created_at: 2026-06-06T04:31:56.866044064+00:00
updated_at: 2026-06-06T04:31:56.866044064+00:00
---

# Description

Test all behavioral aspects of Firebending Lesson in MTG Forge-rs.

Card: cardsfolder/f/firebending_lesson.txt
Set: ATLA / 2025 Standard
Deck: 01 Manfield, 03 Davis Izzet Lessons (2025 WC)

Card text:
  R  Instant — Lesson
  Kicker {4}
  Firebending Lesson deals 2 damage to target creature. If this spell was kicked, it deals 5 damage to that creature instead.

Findings (2026-06-05_#3008(50175e06)):

1. [x] Parses: cost R, Instant Lesson, K:Kicker:4
2. [BROKEN] Targets player when no creatures exist: when ValidTgts$ Creature has no legal targets, the zero controller still casts and targets a player (Zero2), dealing 0 damage. The card should not be castable without a creature target.
3. [PARTIAL] Damage amount: SVar:X:Count$Kicked.5.2 — the kicked/unkicked conditional evaluates to 2 (unkicked) but the targeting-player issue results in 0 damage logged for the first cast.
4. [unverified] Kicker cost option and 5 damage mode

Reproducer:
```sh
./target/release/mtg tui --p1 zero --p2 zero --p1-draw "Firebending Lesson;Island;Island;Mountain;Island;Island;Island" --p2-draw "Island;Island;Island;Island;Island;Island;Island" --seed 42 --verbosity 3 decks/championship/2025/01_manfield_izzet_lessons.dck decks/championship/2025/01_manfield_izzet_lessons.dck 2>&1 | grep -E "Firebending|deals|damage|creature|target"
```

Expected: Not castable when no creatures exist, or targets a creature.
Actual: "Firebending Lesson deals 0 damage to Zero2"

CARD STATUS: BROKEN — targeting bypass (targets player instead of creature), 0 damage to player
