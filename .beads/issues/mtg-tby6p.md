---
title: 'Card Compatibility: Ral, Crackling Wit'
status: open
priority: 3
issue_type: task
created_at: 2026-06-06T04:31:43.825802734+00:00
updated_at: 2026-06-06T04:31:43.825802734+00:00
---

# Description

Test all behavioral aspects of Ral, Crackling Wit in MTG Forge-rs.

Card: cardsfolder/r/ral_crackling_wit.txt
Set: ATLA / 2025 Standard
Deck: 02 Shibata Izzet Lessons (sideboard)

Card text:
  2UR  Legendary Planeswalker — Ral  [4 loyalty]
  Whenever you cast a noncreature spell, put a loyalty counter on Ral, Crackling Wit.
  [+1]: Create a 1/1 blue and red Otter creature token with prowess.
  [-3]: Draw three cards, then discard two cards.
  [-10]: Draw three cards. You get an emblem with "Instant and sorcery spells you cast have storm."

Findings (2026-06-05_#3008(50175e06)):

1. [x] Parses: cost 2UR, Legendary Planeswalker — Ral, enters with 4 loyalty
2. [x] +1 activated ability: creates Otter Token (working)
3. [x] Loyalty activation costs (AddLoyalty/SubLoyalty) work
4. [PARTIAL] Noncreature SpellCast trigger: fires correctly but log says "+1/+1 counter" instead of "gains 1 loyalty" (mtg-gi104). State IS correct.
5. [BROKEN] Otter token Prowess fizzles (mtg-b0igv) — Prowess on Otter tokens does not apply.
6. [unverified] -3 ability (draw 3, discard 2)
7. [unverified] -10 ultimate (storm emblem)

Reproducer:
```sh
./target/release/mtg tui --p1 zero --p2 zero --p1-draw "Ral, Crackling Wit;Island;Island;Mountain;Mountain;Mountain;Mountain" --p2-draw "Island;Island;Island;Island;Island;Island;Island" --seed 42 --verbosity 3 debug/izzet_sideboard_test.dck debug/izzet_sideboard_test.dck 2>&1 | grep -E "Ral|loyalty|Otter|Trigger|counter|planeswalker"
```

Expected: After casting a noncreature spell with Ral on BF: "Ral, Crackling Wit gains 1 loyalty"
Actual: "Ral, Crackling Wit gets a +1/+1 counter (now 1 counters)" (wrong log, correct state)

CARD STATUS: PARTIAL — casts/activates/triggers work; log bug for loyalty trigger (mtg-gi104); Prowess on tokens broken (mtg-b0igv)
