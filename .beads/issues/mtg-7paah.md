---
title: 'Card Compatibility: Pawpatch Formation'
status: open
priority: 3
issue_type: task
created_at: 2026-06-06T04:33:06.715821305+00:00
updated_at: 2026-06-06T04:33:06.715821305+00:00
---

# Description

Test all behavioral aspects of Pawpatch Formation in MTG Forge-rs.

Card: cardsfolder/p/pawpatch_formation.txt
Set: TDM/Tarkir: Dragonstorm
Deck: 04 Henry Temur Otters (mtg-684) — sideboard

Card text:
  {1}{G} Instant
  Choose one —
  • Destroy target creature with flying.
  • Destroy target enchantment.
  • Draw a card. Create a Food token.

Findings (2026-06-06_#3008(50175e06), agent slot04):

1. [x] Parses as {1}{G} Instant with three SP$ Charm choices: WORKING
2. [unverified] Mode 1 (destroy flying creature): ValidTgts$ Creature.withFlying — requires a flying creature target; not tested
3. [unverified] Mode 2 (destroy enchantment): ValidTgts$ Enchantment — requires an enchantment target; not tested
4. [unverified] Mode 3 (draw + food): DB$ Draw + DBToken with c_a_food_sac script — not exercised (AI never cast Pawpatch Formation in observed games)
5. [PARTIAL] Heuristic AI did not cast: even when mana was available, AI passed on Pawpatch Formation. Likely because mode 1+2 require valid targets that weren't present; mode 3 (always castable) should be available but AI may not score 'draw+food' instants without specific conditions.

Reproducer:
```sh
./target/release/mtg tui --p1 heuristic --p2 heuristic --seed 42 --verbosity 2 debug/temur_sideboard_test.dck decks/championship/2025/04_henry_temur_otters.dck
```

CARD STATUS: PARTIAL — parses correctly, charm modes scripted; none directly exercised in gameplay
