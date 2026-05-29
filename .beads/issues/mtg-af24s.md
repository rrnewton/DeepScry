---
title: 'Bug: Charm modal spell modes ignore ValidTgts target restriction'
status: open
priority: 2
issue_type: bug
created_at: 2026-05-29T18:26:42.574449507+00:00
updated_at: 2026-05-29T18:26:42.574449507+00:00
---

# Description

SP$ Charm modal spells do not enforce the per-mode ValidTgts$ target
restriction defined on the chosen mode's SVar sub-ability. The target
selector falls back to a broad category (any permanent/spell), letting the
spell illegally target objects outside the printed restriction.

Discovered (2026-05-29_#2461(53f1d817), compat-oldschool-wave6) on:
- Red Elemental Blast (cardsfolder/r/red_elemental_blast.txt), mtg-536
    A:SP$ Charm | Choices$ DBCounter,DBDestroy
    SVar:DBCounter:DB$ Counter | TargetType$ Spell | ValidTgts$ Card.Blue
    SVar:DBDestroy:DB$ Destroy | ValidTgts$ Permanent.Blue

Reproducer:

```sh
cat > /tmp/rebl.pzl <<'P'
[metadata]
Name:REBL Destroy Blue Permanent
Goal:Win
Turns:3
Difficulty:Easy
Description:REBL should only target BLUE permanents.
[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0hand=Red Elemental Blast
p0library=Mountain; Mountain; Mountain; Mountain
p0graveyard=
p0battlefield=Mountain
p0exile=
p1life=20
p1hand=
p1library=Island; Island; Island; Island
p1graveyard=
p1battlefield=Phantom Monster
p1exile=
P
./target/release/mtg tui --start-state /tmp/rebl.pzl --p1=fixed --p2=zero \
  --p1-fixed-inputs="cast Red Elemental Blast;*;*;*" --p2-fixed-inputs="" \
  --stop-on-choice=8 --seed 42 --verbosity 3
```

Observed (WRONG): the "Destroy target blue permanent" mode targeted the
caster's own RED Mountain and destroyed it:
    Player 1 chooses mode: Destroy target blue permanent.
      -> targeting Mountain (4)
    Red Elemental Blast (3) destroys Mountain (4)

Expected: the only legal target is the opponent's blue creature
(Phantom Monster). A red Mountain is not a blue permanent and is an illegal
target (CR 115.4 / 608.2b).

Root cause (suspected): params_to_charm_effect_with_svars() in
mtg-engine/src/loader/effect_converter.rs builds each ModalMode via
params_to_effect() on the mode's SVar, but the resulting target selection
does not carry the mode's ValidTgts$ colour qualifier into the
target-legality check; the selector defaults to the broad category.

Impact: every modal Charm spell whose modes carry a ValidTgts$ qualifier
(REBL mtg-536, Blue Elemental Blast mtg-487, Pyroblast, Hydroblast, many
Charms). Until fixed these are BROKEN (can hit illegal targets).
