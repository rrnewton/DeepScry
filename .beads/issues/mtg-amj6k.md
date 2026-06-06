---
title: 'Card Compatibility: Willowrush Verge'
status: open
priority: 3
issue_type: task
created_at: 2026-06-06T04:30:28.099735366+00:00
updated_at: 2026-06-06T04:30:28.099735366+00:00
---

# Description

Test all behavioral aspects of Willowrush Verge in MTG Forge-rs.

Card: cardsfolder/w/willowrush_verge.txt
Set: DSK (Aetherdrift/Duskmourn)
Deck: 04 Henry Temur Otters (mtg-684)

Card text:
  (no cost) Land
  {T}: Add {U}.
  {T}: Add {G}. Activate only if you control a Forest or an Island.

Findings (2026-06-06_#3008(50175e06), agent slot04):

1. [x] Parses as land with two mana abilities
2. [x] First mana ability (tap for U): WORKING — observed in game: 'Tap Willowrush Verge for {U}' (seed 43)
3. [x] Second mana ability (tap for G, conditional): PARTIAL — the card parses the IsPresent condition but gameplay did not exercise the gate extensively. The mana ability produces G correctly; the conditional restriction (only if Forest or Island) needs deeper verification.
4. [x] Plays as a land (plays for free on T1): WORKING — observed in multiple game logs
5. [N/A] ETB trigger: card has no ETB trigger

Reproducer:
```sh
./target/release/mtg tui --p1 zero --p2 zero --p1-draw 'Willowrush Verge;Breeding Pool;Stomping Ground;Breeding Pool;Stomping Ground' --p2-draw 'Island;Island;Island;Island;Island;Island;Island' --seed 42 --verbosity 3 decks/championship/2025/04_henry_temur_otters.dck decks/championship/2025/01_manfield_izzet_lessons.dck
```

Expected log evidence:
```
Zero1 plays Willowrush Verge
Tap Willowrush Verge for {U}
```

CARD STATUS: PARTIAL — basic mana production WORKING; conditional {G} ability not deeply verified
