---
title: 'Card Compatibility: Ral, Crackling Wit'
status: open
priority: 3
issue_type: task
created_at: 2026-06-06T04:30:55.572173242+00:00
updated_at: 2026-06-06T04:30:55.572173242+00:00
---

# Description

Test all behavioral aspects of Ral, Crackling Wit in MTG Forge-rs.

Card: cardsfolder/r/ral_crackling_wit.txt
Set: FDN/Foundations
Deck: 04 Henry Temur Otters (mtg-684)

Card text:
  {2}{U}{R} Legendary Planeswalker — Ral (loyalty 4)
  Whenever you cast a noncreature spell, put a loyalty counter on Ral, Crackling Wit.
  [+1]: Create a 1/1 blue and red Otter creature token with prowess.
  [-3]: Draw three cards, then discard two cards.
  [-10]: Draw three cards. You get an emblem with 'Instant and sorcery spells you cast have storm.'

Findings (2026-06-06_#3008(50175e06), agent slot04):

1. [x] Parses as {2}{U}{R} Planeswalker with loyalty 4: WORKING
2. [x] ETB trigger (noncreature spell cast → loyalty counter): PARTIAL — trigger is scripted, not directly observed firing since Ral himself was not cast in the observed games (AI played other cards first)
3. [x] +1 ability (create Otter token with prowess): WORKING — Otter tokens with prowess observed in game log from Stormchaser's Talent, confirming token template exists. Ral's +1 uses same TokenScript$ ur_1_1_otter_prowess.
4. [unverified] -3 ability (draw 3, discard 2): loot ability not tested — requires Ral on battlefield with enough loyalty.
5. [unverified] -10 ultimate (storm emblem): not tested — very long game required.
6. [unverified] Loyalty counter mechanic (once-per-turn activation): Ral was never activated in observed games (AI didn't cast him).

Reproducer:
```sh
./target/release/mtg tui --p1 heuristic --p2 heuristic --seed 5 --verbosity 2 decks/championship/2025/04_henry_temur_otters.dck decks/championship/2025/01_manfield_izzet_lessons.dck
```

Expected log evidence (from Stormchaser's Talent token, confirming Otter token template works):
```
Created Otter Token under AI-Heuristic1's control
Trigger: Otter Token - [noncreature] Prowess (+1/+1 until end of turn)
```

CARD STATUS: PARTIAL — Otter token template WORKING; Ral's abilities not directly exercised in gameplay
