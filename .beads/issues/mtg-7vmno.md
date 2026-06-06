---
title: 'Card Compatibility: Annul'
status: open
priority: 3
issue_type: task
created_at: 2026-06-06T04:33:32.506447565+00:00
updated_at: 2026-06-06T04:36:04.815356684+00:00
---

# Description

Test all behavioral aspects of Annul in MTG Forge-rs.

Card: cardsfolder/a/annul.txt
Set: Classic/Urza's Saga
Deck: 04 Henry Temur Otters (mtg-684) — sideboard

Card text:
  {U} Instant
  Counter target artifact or enchantment spell.

Findings (2026-06-06_#3008(50175e06), agent slot04):

1. [x] Parses as {U} Instant: WORKING
2. [x] Casts and resolves: WORKING
3. [BROKEN] Type restriction not enforced: Observed 'Annul (49) counters Ral, Crackling Wit (81)' — Ral is a Planeswalker spell on the stack, NOT an artifact or enchantment. The ValidTgts$ Artifact,Enchantment restriction is not passed to the counter targeting engine.
   Root cause: Effect::CounterSpell only stores required_color from ValidTgts, ignoring type list. See mtg-h0jqf.

Reproducer:
```sh
./target/release/mtg tui --p1 heuristic --p2 heuristic --seed 42 --verbosity 2 debug/temur_sideboard_test.dck decks/championship/2025/04_henry_temur_otters.dck
```

Expected log evidence (BUG):
```
Annul (49) counters Ral, Crackling Wit (81)
```
(Ral is a Planeswalker, NOT an artifact or enchantment — should be an invalid target)

Bug: mtg-h0jqf

CARD STATUS: BROKEN — artifact/enchantment type restriction not enforced; counters any spell regardless of type
