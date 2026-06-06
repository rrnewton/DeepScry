---
title: 'Card Compatibility: Pyroclasm'
status: open
priority: 3
issue_type: task
created_at: 2026-06-06T04:33:14.760636568+00:00
updated_at: 2026-06-06T04:33:14.760636568+00:00
---

# Description

Test all behavioral aspects of Pyroclasm in MTG Forge-rs.

Card: cardsfolder/p/pyroclasm.txt
Set: Classic/Tempest
Deck: 04 Henry Temur Otters (mtg-684) — sideboard

Card text:
  {1}{R} Sorcery
  Pyroclasm deals 2 damage to each creature.

Findings (2026-06-06_#3008(50175e06), agent slot04):

1. [x] Parses as {1}{R} Sorcery: WORKING
2. [x] Casts and resolves: WORKING — 'AI-Heuristic1 casts Pyroclasm (57) / Pyroclasm (57) resolves' (sideboard test deck, seed 42)
3. [x] Deals 2 damage to each creature: WORKING — 'Pyroclasm (57) deals 2 damage to each matching creature' (sideboard test game)
4. [N/A] Targeting: no targets (affects all creatures)
5. [N/A] Alternative costs: none

Reproducer:
```sh
./target/release/mtg tui --p1 heuristic --p2 heuristic --seed 42 --verbosity 2 debug/temur_sideboard_test.dck decks/championship/2025/04_henry_temur_otters.dck
```

Expected log evidence:
```
AI-Heuristic1 casts Pyroclasm (57) (putting on stack)
Pyroclasm (57) resolves
Pyroclasm (57) deals 2 damage to each matching creature
```

CARD STATUS: WORKING — deals 2 damage to each creature as printed
