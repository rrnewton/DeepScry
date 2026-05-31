---
title: 'Bug: Fireball multi-target DivideEvenly + RaiseCost incomplete (single-target X-damage OK)'
status: open
priority: 3
issue_type: task
created_at: 2026-05-31T00:22:29.827028377+00:00
updated_at: 2026-05-31T00:22:29.827028377+00:00
---

# Description

Fireball (cardsfolder/f/fireball.txt, mtg-505) single-target X damage WORKS (cast with N mana => X = N-1 damage to one target, verified seed 42). The MULTI-target "divide X damage evenly, rounded down, among any number of targets" mode plus the "costs {1} more per target beyond the first" static do NOT work correctly.

Script:
  S:Mode\$ RaiseCost | ValidCard\$ Card.Self | Amount\$ IncreaseCost | Relative\$ True (costs {1} more per extra target)
  A:SP\$ DealDamage | ValidTgts\$ Any | NumDmg\$ X | TargetMin\$ 0 | TargetMax\$ MaxTargets | DivideEvenly\$ RoundedDown
  SVar:X:Count\$xPaid
  SVar:MaxTargets:SVar\$MaxPlayers/Plus.MaxPermanents
  SVar:IncreaseCost:TargetedObjects\$Amount/Minus.1

Observed (fixed-inputs, 5 Mountains, two Grizzly Bears on opp board): casting
Fireball logged "→ targeting Grizzly Bears (19)" but then resolved
"Fireball deals 4 damage to Player 2" — i.e. the chosen creature target was
dropped and the damage went to a player. The DivideEvenly split across
multiple targets and the RaiseCost-per-extra-target are not implemented.

Engine work required:
1. Multi-target collection for SP\$ DealDamage with TargetMax\$ <SVar> (variable
   max targets) and DivideEvenly\$ RoundedDown (CR 601.2d announce division).
2. Mode\$ RaiseCost | Relative\$ True with Amount keyed on number of targets
   chosen (TargetedObjects\$Amount/Minus.1) — cost increases at announcement.

Until then Fireball is PARTIAL (single-target X-burn works; the divide mode is
the rarer use). Found during compat-wave14-jeskai (Jeskai Aggro, mtg-561).
