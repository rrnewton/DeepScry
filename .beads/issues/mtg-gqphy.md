---
title: 'Card Compatibility: Gran-Gran'
status: open
priority: 3
issue_type: task
created_at: 2026-06-06T04:30:27.761144771+00:00
updated_at: 2026-06-06T04:30:27.761144771+00:00
---

# Description

Test all behavioral aspects of Gran-Gran in MTG Forge-rs.

Card: cardsfolder/g/gran_gran.txt
Set: ATLA (Avatar: The Last Airbender)
Deck: 01 Manfield Izzet Lessons (2025 WC), 03 Davis Izzet Lessons (2025 WC)

Card text:
  U  1/2  Legendary Creature — Human Peasant Ally
  Whenever Gran-Gran becomes tapped, draw a card, then discard a card.
  Noncreature spells you cast cost {1} less to cast as long as there are three or more Lesson cards in your graveyard.

Findings (2026-06-05_#3008(50175e06)):

1. [x] Parses correctly: cost U, P/T 1/2, Legendary Creature — Human Peasant Ally
2. [x] ETB: enters as 1/2 creature
3. [unverified] Tap trigger: T:Mode$ Taps — fires when tapped. Not directly observed in zero-controller game since Gran-Gran rarely attacks. TriggerEvent::Taps is registered in the engine.
4. [PARTIAL] Cost reduction: S:Mode$ ReduceCost | IsPresent$ Lesson.YouOwn | PresentZone$ Graveyard | PresentCompare$ GE3. The IsPresent with PresentZone$ Graveyard likely uses static condition evaluation. The Count$ValidGraveyard bug (mtg-cedrg) may affect this if the condition evaluates graveyard count.
5. [N/A] No keywords

Reproducer:
```sh
./target/release/mtg tui --p1 zero --p2 zero --p1-draw "Gran-Gran;Island;Island;Island;Island;Island;Island" --p2-draw "Island;Island;Island;Island;Island;Island;Island" --seed 42 --verbosity 3 decks/championship/2025/01_manfield_izzet_lessons.dck decks/championship/2025/01_manfield_izzet_lessons.dck 2>&1 | grep -E "Gran-Gran|draw|discard|1/2"
```

Expected log evidence:
```
Gran-Gran (18) enters the battlefield as a 1/2 creature
```

CARD STATUS: PARTIAL — enters/casts working; tap trigger unverified in game log; cost-reduction condition depends on graveyard count (blocked by mtg-cedrg)
