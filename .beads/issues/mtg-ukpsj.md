---
title: 'Card Compatibility: Disdainful Stroke'
status: closed
priority: 3
issue_type: task
created_at: 2026-06-06T04:33:44.485685213+00:00
updated_at: 2026-06-06T08:35:34.563962982+00:00
---

# Description

Test all behavioral aspects of Disdainful Stroke in MTG Forge-rs.

Card: cardsfolder/d/disdainful_stroke.txt
Set: Classic/Khans of Tarkir
Deck: 04 Henry Temur Otters (mtg-684) — sideboard

Card text:
  {1}{U} Instant
  Counter target spell with mana value 4 or greater.

Findings (2026-06-06_#3008(50175e06), agent slot04):

1. [x] Parses as {1}{U} Instant: WORKING
2. [x] Casts and resolves: WORKING — 'AI-Heuristic1 casts Disdainful Stroke (54) / Disdainful Stroke (54) resolves' (sideboard test deck, seed 42)
3. [BROKEN] CMC filter not enforced: Disdainful Stroke countered Thundertrap Trainer (CMC 2) and Badgermole Cub (CMC 2), both below the required CMC 4 threshold. The ValidTgts$ Card.cmcGE4 modifier is silently dropped by TargetRestriction::parse().
   Root cause: see mtg-h0jqf (ValidTgts CMC/nonCreature modifiers silently dropped)

Reproducer:
```sh
./target/release/mtg tui --p1 heuristic --p2 heuristic --seed 42 --verbosity 3 debug/temur_sideboard_test.dck decks/championship/2025/04_henry_temur_otters.dck
```

Expected log evidence (BUG):
```
Disdainful Stroke (54) counters Thundertrap Trainer (117)
```
(Thundertrap Trainer CMC 2 should NOT be a legal target; should not be counterable by Disdainful Stroke)

Bug: mtg-h0jqf

CARD STATUS: BROKEN — CMC ≥ 4 restriction not enforced; counters any spell regardless of mana value

## Fix (2026-06-06, integration via slot04):
mtg-h0jqf fixed — Disdainful Stroke's cmcGE4 restriction is enforced. Only counters spells with MV≥4.

CARD STATUS: WORKING
