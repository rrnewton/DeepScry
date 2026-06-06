---
title: 'Card Compatibility: Negate'
status: open
priority: 3
issue_type: task
created_at: 2026-06-06T04:36:22.125643036+00:00
updated_at: 2026-06-06T04:36:22.125643036+00:00
---

# Description

Test all behavioral aspects of Negate in MTG Forge-rs.

Card: cardsfolder/n/negate.txt
Set: Classic/Morningtide
Deck: 04 Henry Temur Otters (mtg-684) — sideboard

Card text:
  {1}{U} Instant
  Counter target noncreature spell.

Findings (2026-06-06_#3008(50175e06), agent slot04):

1. [x] Parses as {1}{U} Instant: WORKING
2. [unverified] Noncreature restriction: ValidTgts$ Card.nonCreature modifier. Based on bug mtg-h0jqf, the nonCreature modifier is likely silently dropped in TargetRestriction::parse(), meaning Negate may counter creature spells. Not directly tested.
3. [unverified] Casts and resolves in gameplay: not exercised in test session

Bug: mtg-h0jqf (ValidTgts type modifiers dropped)

Reproducer:
```sh
./target/release/mtg tui --p1 heuristic --p2 heuristic --seed 42 --verbosity 2 debug/temur_sideboard_test.dck decks/championship/2025/04_henry_temur_otters.dck
```

CARD STATUS: PARTIAL — parses correctly; noncreature restriction likely broken (mtg-h0jqf)
