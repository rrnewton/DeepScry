---
title: 'Card Compatibility: Accumulate Wisdom'
status: open
priority: 3
issue_type: task
created_at: 2026-06-06T04:30:56.090605959+00:00
updated_at: 2026-06-06T04:30:56.090605959+00:00
---

# Description

Test all behavioral aspects of Accumulate Wisdom in MTG Forge-rs.

Card: cardsfolder/a/accumulate_wisdom.txt
Set: ATLA / 2025 Standard
Deck: 01 Manfield, 02 Shibata, 03 Davis Izzet Lessons (2025 WC)

Card text:
  1U  Instant — Lesson
  Look at the top three cards of your library. Put one of those cards into your hand and the rest on the bottom of your library in any order. Put each of those cards into your hand instead if there are three or more Lesson cards in your graveyard.

Findings (2026-06-05_#3008(50175e06)):

1. [x] Parses: cost 1U, Instant Lesson
2. [BROKEN] ChangeNum$ X (dynamic) broken: SVar:X:Count$Compare Y GE3.3.1 with Y=Count$ValidGraveyard Lesson.YouOwn. Because Count$ValidGraveyard is not implemented (mtg-cedrg), Y=0, Compare 0 GE3 is false, so ChangeNum resolves to Fixed(0) but DigMultiple fallback sets it to dig_count (3). Result: ALWAYS puts ALL 3 cards into hand regardless of how many Lessons are in graveyard.
3. [x] Looks at top 3 cards - log shows "Zero1 looks at the top 3 cards of their library"
4. [unverified] Conditional 3-card vs 1-card behavior (blocked by mtg-cedrg)

Also note the log message says "digs N card(s) from opponent's library to hand" which seems wrong (it digs from OWN library).

Reproducer:
```sh
./target/release/mtg tui --p1 zero --p2 zero --p1-draw "Accumulate Wisdom;Island;Island;Island;Island;Island;Island" --p2-draw "Island;Island;Island;Island;Island;Island;Island" --seed 42 --verbosity 3 decks/championship/2025/01_manfield_izzet_lessons.dck decks/championship/2025/01_manfield_izzet_lessons.dck 2>&1 | grep -E "Accumulate|looks at|puts|hand|bottom"
```

Expected: puts 1 into hand when graveyard has fewer than 3 Lessons.
Actual: puts ALL 3 into hand (ChangeNum$ X parsing falls back to dig_count=3).

CARD STATUS: BROKEN — conditional 3-vs-1 behavior always fires 3 (Count$ValidGraveyard bug mtg-cedrg)
