---
title: 'Bug: AB$ Tap auto-targets its own source (Icy Manipulator)'
status: open
priority: 3
issue_type: bug
created_at: 2026-05-29T16:33:05.044773850+00:00
updated_at: 2026-05-29T16:33:05.044773850+00:00
---

# Description

When activating an AB$ Tap ability with ValidTgts$ Artifact,Creature,Land (Icy Manipulator), both the fixed and heuristic controllers auto-select the FIRST legal target, which is the source artifact itself. The ability resolves but taps Icy Manipulator instead of the intended opponent permanent, making it useless.

Observed (puzzle: P0 Icy Manipulator + Moxen, P1 Serra Angel; seed 42, both --p1=fixed and --p1=heuristic):
  Icy Manipulator activates ability: Tap target artifact, creature, or land.
    -> targeting Icy Manipulator (3)

The Tap EFFECT is correct (it taps the chosen target); the bug is in TARGET
SELECTION for activated abilities whose ValidTgts includes the source's own
type. Two improvements possible:
1. Controllers should prefer an opponent's permanent (or at least not the
   ability's own source / a permanent that is pointless to tap) when choosing
   a Tap target.
2. Provide a way for fixed-input scripts to specify the activated-ability
   target (the 'target X' fixed input is consumed at cast for spells but does
   not reach activated-ability target selection here).

Impact: Icy Manipulator (mtg-511) classified PARTIAL — mana/tap mechanics work
but the AI cannot aim it usefully. Likely affects other targeted tap/untap
activated abilities (e.g. Relic Barrier, Winter Orb-style effects).

Found during the 1994 Old School Mono Black Rogerbrand deck pass (mtg-560).
