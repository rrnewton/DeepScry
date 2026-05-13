---
title: 'Card Compatibility: Animate Dead'
status: open
priority: 2
issue_type: task
created_at: 2026-05-13T02:20:10.693792605+00:00
updated_at: 2026-05-13T16:26:12.069728929+00:00
---

# Description

ADVANCED FIX 2026-05-13 (counter-fix branch).

Set: LEA (mtg-3c7c63)
Deck: rogue_rogerbrand (mtg-526f25)
Card script: cardsfolder/a/animate_dead.txt

## Behavioural status

1. [x] Castable for {1}{B} as Sorcery-speed Aura
2. [x] Targets a creature card in any GRAVEYARD
3. [x] Aura targeting picks the graveyard creature
4. [x] Spell resolves on the stack
5. [x] Reanimation: targeted creature returns from graveyard to battlefield under aura controller (NEW — was BROKEN)
6. [partial] DBAnimate keyword swap not implemented; workaround in attach_aura strips `.inZone<X>` qualifier
7. [x] DBAttach attaches Animate Dead to the reanimated creature (NEW)
8. [x] -1/-0 continuous effect via Affected$ Creature.EnchantedBy (NEW — fires automatically once attached)
9. [BROKEN] DBDelay sacrifice trigger when Animate Dead leaves the battlefield (tracked in mtg-abfad9)
10. [BROKEN] Cleanup of remembered list (low impact; tracked in mtg-abfad9)

## What changed

- mtg-engine/src/game/actions/mod.rs: new `apply_etb_counters` + `reanimate_aura_target` helpers wired into `play_land` and `resolve_spell_finalize`.
- mtg-engine/src/game/state.rs: `find_card_zone` is now `pub` so callers can detect "Aura targets a non-battlefield card".
- mtg-engine/src/game/actions/mod.rs (attach_aura): `Enchant:Creature.inZoneGraveyard`-style restriction is normalized by stripping the `.inZone<X>` qualifier so post-reanimation attach succeeds.

Verified e2e: Animate Dead reanimating Triskelion produces a 3/4 (1/1 base + 3 P1P1 - 1/-0) with both effects visibly applied; `tests/animate_dead_reanimate_triskelion_e2e.sh`.

CARD STATUS: WORKING for the core reanimation use case. DBDelay sacrifice-on-leave + DBAnimate keyword rewrite remain BROKEN — see mtg-abfad9.
