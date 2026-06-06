---
title: 'Card Compatibility: Essence Scatter'
status: open
priority: 3
issue_type: task
created_at: 2026-06-06T04:36:31.132248464+00:00
updated_at: 2026-06-06T04:36:31.132248464+00:00
---

# Description

Test all behavioral aspects of Essence Scatter in MTG Forge-rs.

Card: cardsfolder/e/essence_scatter.txt
Set: Classic/Magic 2013
Deck: 04 Henry Temur Otters (mtg-684) — sideboard

Card text:
  {1}{U} Instant
  Counter target creature spell.

Findings (2026-06-06_#3008(50175e06), agent slot04):

1. [x] Parses as {1}{U} Instant: WORKING
2. [x] Casts and resolves: WORKING — 'AI-Heuristic1 casts Essence Scatter (60) / Essence Scatter (60) resolves / Essence Scatter (60) counters Torch the Tower (120)' (sideboard test deck, seed 42)
   NOTE: Torch the Tower is an instant (not a creature spell), meaning the creature restriction is ALSO not enforced here.
3. [BROKEN] Creature restriction not enforced: Essence Scatter countered Torch the Tower (an instant, not a creature spell). The ValidTgts$ Creature is not passed to the counter targeting engine. Same root cause as mtg-h0jqf.

Reproducer:
```sh
./target/release/mtg tui --p1 heuristic --p2 heuristic --seed 42 --verbosity 2 debug/temur_sideboard_test.dck decks/championship/2025/04_henry_temur_otters.dck
```

Expected log evidence (BUG):
```
Essence Scatter (60) counters Torch the Tower (120)
```
(Torch the Tower is an instant — should be an invalid target for Essence Scatter)

Bug: mtg-h0jqf

CARD STATUS: BROKEN — creature-only restriction not enforced; counters non-creature spells
