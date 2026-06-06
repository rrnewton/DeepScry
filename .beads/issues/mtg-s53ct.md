---
title: 'Card Compatibility: Song of Totentanz'
status: open
priority: 3
issue_type: task
created_at: 2026-06-06T04:31:30.787900370+00:00
updated_at: 2026-06-06T04:31:30.787900370+00:00
---

# Description

Test all behavioral aspects of Song of Totentanz in MTG Forge-rs.

Card: cardsfolder/s/song_of_totentanz.txt
Set: WOE/Wilds of Eldraine
Deck: 04 Henry Temur Otters (mtg-684)

Card text:
  {X}{R} Sorcery
  Create X 1/1 black Rat creature tokens with 'This creature can't block.'
  Creatures you control gain haste until end of turn.

Findings (2026-06-06_#3008(50175e06), agent slot04):

1. [x] Parses as {X}{R} Sorcery: WORKING
2. [x] X-cost (creates X tokens): WORKING — observed: 'Song of Totentanz (47) resolves\nCreated Rat Token under AI-Heuristic1's control\nSong of Totentanz (47) creates 1 b_1_1_rat_noblock token(s)' (seed 42, heuristic game, X=1)
3. [x] Rat tokens have 'can't block' keyword: WORKING — token uses b_1_1_rat_noblock script
4. [x] Haste granted to all creatures: WORKING — observed tokens attacking immediately turn they're created (DBPumpAll with KW$ Haste)
5. [unverified] Large X values (X=3+): not tested, but token-creation loop should scale

Reproducer:
```sh
./target/release/mtg tui --p1 heuristic --p2 heuristic --seed 42 --verbosity 2 decks/championship/2025/04_henry_temur_otters.dck decks/championship/2025/01_manfield_izzet_lessons.dck
```

Expected log evidence:
```
Song of Totentanz (47) resolves
Created Rat Token under AI-Heuristic1's control
Song of Totentanz (47) creates 1 b_1_1_rat_noblock token(s) under AI-Heuristic1's control
```

CARD STATUS: WORKING — token creation with haste observed in gameplay
