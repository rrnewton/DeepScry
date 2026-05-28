---
title: 'Combat: blocked creature survives lethal combat damage (Serra Angel 4/4 didn''t kill BoP 0/1)'
status: open
priority: 2
issue_type: task
created_at: 2026-05-28T03:43:27.225367370+00:00
updated_at: 2026-05-28T03:43:47.791765263+00:00
---

# Description

CLUSTER: gameplay. ALARMING — fundamental combat-damage bug, likely affects many cards (not Serra-specific). Per-card: mtg-c6dfe3 (Serra Angel).

Live playtest: P1 blocked P2's Birds of Paradise (0/1) with Serra Angel (4/4). BoP did NOT die. Log excerpt:

  P2 declares Birds of Paradise (71) (0/1) as attacker
  P1 declares Serra Angel (52) as blocker for Birds of Paradise (71)
  >>> Turn 19 - P1 20 (P2 8) <<<
  P1 draws Swords to Plowshares (58)
  P1 declares Serra Angel (52) (4/4) as attacker   <- still alive next turn, BoP survived

Expected (CR 510.2): Serra Angel deals 4 combat damage to the blocked Birds of Paradise; 4 >= toughness 1 → BoP is destroyed as a state-based action (CR 704.5g). It wasn't.

Hypotheses to investigate:
1. A blocker deals no combat damage to the attacker it blocks (combat damage only assigned attacker->blocker, not blocker->attacker?).
2. State-based lethal-damage check not run after combat damage for the attacking creature.
3. The 0/1 attacker's damage-marked vs toughness comparison is wrong.

This is core combat. Write a minimal puzzle (4/4 blocks 0/1, assert attacker dies) and a regression e2e. Root-cause in the combat-damage assignment + SBA code. MTG rules review required.
