---
title: 'Card Compatibility: Iroh''s Demonstration'
status: open
priority: 3
issue_type: task
created_at: 2026-06-06T04:33:26.749421804+00:00
updated_at: 2026-06-06T04:33:26.749421804+00:00
---

# Description

Test all behavioral aspects of Iroh's Demonstration in MTG Forge-rs.

Card: cardsfolder/i/irohs_demonstration.txt
Set: Avatar: The Last Airbender crossover
Deck: 04 Henry Temur Otters (mtg-684) — sideboard; also in Izzet Lessons decks

Card text:
  {1}{R} Sorcery — Lesson
  Choose one —
  • Iroh's Demonstration deals 1 damage to each creature your opponents control.
  • Iroh's Demonstration deals 4 damage to target creature.

Findings (2026-06-06_#3008(50175e06), agent slot04):

1. [x] Parses as {1}{R} Sorcery — Lesson with SP$ Charm choices: WORKING
2. [x] Casts and resolves as part of Izzet Lessons deck (seed 42 heuristic game, P2 cast it): WORKING
3. [PARTIAL] Network log extra-lines bug: existing issue mtg-381 notes that network mode logs extra 'takes N damage (total: N)' lines for multi-target damage from Iroh's Demonstration. This is a network-only log-gap, not a gameplay correctness issue.
4. [unverified] Mode 1 (1 damage to each opponent creature): DamageAll to Creature.OppCtrl — not exercised in this test session
5. [unverified] Mode 2 (4 damage to target creature): DealDamage to Creature — not directly tested here

References: mtg-381 (network extra-lines bug)

Reproducer:
```sh
./target/release/mtg tui --p1 heuristic --p2 heuristic --seed 42 --verbosity 2 decks/championship/2025/04_henry_temur_otters.dck decks/championship/2025/01_manfield_izzet_lessons.dck
```

CARD STATUS: PARTIAL — parses and casts as Lesson; network extra-lines bug (mtg-381); modes not directly verified in this session
