---
title: 'Bug: SP$ DealDamage + SubAbility$ Effect chain double-resolves + ReplaceDyingDefined (exile instead) unimplemented'
status: open
priority: 2
issue_type: task
created_at: 2026-05-31T01:49:00.355729480+00:00
updated_at: 2026-05-31T01:49:00.355729480+00:00
---

# Description

Two gaps surfaced by Disintegrate (`SP$ DealDamage | ValidTgts$ Any | NumDmg$ X | SubAbility$ DBEffect | ReplaceDyingDefined$ ThisTargetedCard.Creature`) in the wave-16 robots sweep (mtg-559, sideboard):

1. DOUBLE DAMAGE: with a single clean cast targeting a creature, Disintegrate deals X to the creature AND ALSO X to a player. The targeted DealDamage appears to resolve once against the chosen creature and once against a default (player) target — the SubAbility$ DBEffect chain / target reuse mis-resolves. Likely related to the in-stack SubAbility resolution path (cf. mtg-559 in-stack re-entry class).

2. EXILE-INSTEAD + CANT-REGEN unimplemented: `ReplaceDyingDefined$` (if the creature would die, exile it instead) and the `Mode$ CantRegenerate` static from the DBEffect SVar are not applied — the creature goes to the graveyard normally.

Reproducer:
```sh
./target/release/mtg tui --start-state /tmp/disint.pzl \
  --p1-fixed-inputs='cast Disintegrate;3;Grizzly Bears' --p1=fixed --p2=zero \
  --seed 42 --verbosity 3 --stop-when-fixed-exhausted
## (p0 hand=Disintegrate, 4 Mountains on bf; p1 bf=Grizzly Bears)
```
Observed: 'Grizzly Bears takes 3 damage' AND 'Disintegrate deals 3 damage to Player 2' (wrong), then 'goes to graveyard' (should exile).

FIX DIRECTION: (a) fix DealDamage+SubAbility target binding so a single targeted DealDamage hits ONLY the chosen target; (b) implement ReplaceDyingDefined (a death-replacement that exiles instead of graveyard for the remembered creature this turn), reusing the finality-counter exile-instead path in state.rs:1843; (c) wire the CantRegenerate static. Defer — multi-part.
