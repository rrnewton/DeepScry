---
title: 'Bug: ValidTgts modifiers cmcGE/cmcLE and nonCreature silently dropped in CounterSpell targets'
status: open
priority: 2
issue_type: task
created_at: 2026-06-06T04:35:02.781771913+00:00
updated_at: 2026-06-06T04:35:52.889091574+00:00
---

# Description

Targeting restrictions in ValidTgts$ for CounterSpell effects that use CMC filters (cmcGE4, cmcLE3), type modifiers (nonCreature, nonLand), or subtype restrictions (Artifact, Enchantment) are silently dropped, causing incorrect targeting.

Confirmed behaviors:
1. Disdainful Stroke (ValidTgts$ Card.cmcGE4) counters spells with CMC < 4 (observed: countered Thundertrap Trainer CMC 2, Badgermole Cub CMC 2)
2. Annul (ValidTgts$ Artifact,Enchantment) counters Planeswalker spells — 'Annul (49) counters Ral, Crackling Wit (81)' (Ral is a Planeswalker, not Artifact/Enchantment)
3. Negate (ValidTgts$ Card.nonCreature) likely allows countering creature spells — untested

Root cause (TWO separate gaps):
A) TargetRestriction::parse() in mtg-engine/src/core/effects.rs silently discards unknown modifiers via _ => {} including cmcGE<N>, cmcLE<N>, nonCreature, nonLand.
B) Effect::CounterSpell only carries required_color from ValidTgts (effect_converter.rs line 391-393). The full TargetRestriction (types, mana value range, nonCreature, etc.) is never stored in the CounterSpell effect or passed to the targeting engine.

Fix requires:
1. Effect::CounterSpell struct needs to carry the full TargetRestriction, not just required_color.
2. TargetRestriction::parse() needs to handle cmcGE<N>, cmcLE<N>, nonCreature, nonLand modifiers properly.
3. Counter targeting logic must enforce TargetRestriction when building valid_targets on the stack.

Cards directly affected:
- Disdainful Stroke (cmcGE4) — BROKEN (mtg-ukpsj)
- Annul (Artifact,Enchantment) — BROKEN (mtg-7vmno)
- Negate (Card.nonCreature) — likely BROKEN (mtg-gavrg)
- Essence Scatter (Creature) — likely BROKEN if nonCreature is not enforced
- Any future counter spell with ValidTgts restrictions beyond color

Reproducer (Disdainful Stroke CMC bypass):
```sh
./target/release/mtg tui --p1 heuristic --p2 heuristic --seed 42 --verbosity 2 debug/temur_sideboard_test.dck decks/championship/2025/04_henry_temur_otters.dck
```

Expected: Disdainful Stroke should NOT counter Thundertrap Trainer (CMC 2). Annul should NOT counter Ral, Crackling Wit.
Actual: Both violations observed in the same game log.
