---
title: 'Bug: SP$ FlipOntoBattlefield + DBDamageTouched (Falling Star) mis-routes to destroy; needs deterministic damage model'
status: open
priority: 4
issue_type: bug
created_at: 2026-05-31T17:32:01.370480433+00:00
updated_at: 2026-05-31T17:32:01.370480433+00:00
---

# Description

Falling Star (cardsfolder/f/falling_star.txt, mtg-503, Troll Disk sideboard mtg-562) is a dexterity 'flip onto the play area' card:

  A:SP$ FlipOntoBattlefield | SubAbility$ DBDamageTouched | AILogic$ DamageCreatures
  SVar:DBDamageTouched:DB$ DamageAll | ValidCards$ Creature.IsRemembered | NumDmg$ 3 ...
  SVar:DBTapAllDamaged:DB$ TapAll | ValidCards$ Creature.IsRemembered+DamagedBy ...

TWO engine gaps:
1. The SP$ FlipOntoBattlefield handler reuses Chaos-Orb's destroy-touched path,
   so Falling Star wrongly DESTROYS a land ('Falling Star destroys Mountain')
   instead of dealing damage. The DamageAll branch must be routed distinctly
   from the destroy branch.
2. ValidCards$ Creature.IsRemembered matches the creatures the card physically
   'landed on', which a digital engine can't simulate. TargetRestriction
   .requires_remembered currently returns matches()==false, so the remembered
   set is empty and 3-damage-to-each-landed-creature never happens.

NEXT STEP: pick a deterministic, replay-safe, native==WASM model for the
FlipOntoBattlefield damage variant (e.g. AI/seeded-random pick of N creatures,
or 'all creatures' as a simplification — Chaos Orb mtg-392 chose one-opponent-
permanent). It must NOT destroy lands, must route DamageAll/TapAll distinctly
from Chaos Orb's destroy path, and must keep the gamelog clean (no Debug dump).

Related: mtg-392 (Chaos Orb FlipOntoBattlefield self-target). Blocks closing
mtg-503 (Falling Star) to WORKING.
