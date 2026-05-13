---
title: 'Bug: Animate Dead ETB-reanimate trigger not firing'
status: open
priority: 3
issue_type: bug
created_at: 2026-05-13T02:45:16.381440871+00:00
updated_at: 2026-05-13T16:25:43.906091724+00:00
---

# Description

Animate Dead's ETB self-trigger T:Mode$ ChangesZone | Origin$ Any | Destination$ Battlefield | ValidCard$ Card.Self | IsPresent$ Card.StrictlySelf | Execute$ TrigReanimate is not fully wired through the trigger system. Status:

## Status: PARTIALLY FIXED in fix/animate-dead-counters branch (2026-05-13)

The user-visible reanimation flow now works for the common case via an inline implementation in `resolve_spell_finalize → reanimate_aura_target`:
  * Aura whose chosen target is in a graveyard skips the immediate "attach to battlefield" path.
  * Helper moves the graveyard creature to battlefield under the Aura's controller (honours `GainControl$ True`).
  * Applies etbCounter + ETB triggers on the reanimated creature.
  * Attaches the Aura post-reanimation.
  * The continuous -1/-0 effect (Affected$ Creature.EnchantedBy) Just Works once attached.

Verified: Animate Dead → Triskelion (graveyard) yields Triskelion 3/4 with three +1/+1 counters and Animate Dead attached. Activated ping ability + lethal-self-damage flow both work; SBA cleans up Animate Dead after Triskelion dies.

## Still missing (deferred):
1. DBDelay: the delayed trigger that sacrifices the reanimated creature when Animate Dead leaves the battlefield. Need DB$ DelayedTrigger handler with RememberObjects$ RememberedLKI.
2. DBAnimate keyword swap: real Java rewrites the Aura's Enchant restriction from "creature card in a graveyard" to "creature put onto the battlefield with CARDNAME". Workaround: attach_aura strips the `.inZone<X>` qualifier so the post-reanimation attach succeeds. Edge case: blink effects that re-target the Aura would need the proper rewrite.
3. Generic DB$ ChangeZone Graveyard→Battlefield with `Defined$ Enchanted` in effect_converter.rs. Without this, other reanimation cards (Dance of the Dead, Spellweaver Volute, Reanimate-the-sorcery) still don't work via the trigger system. Tracked separately for follow-up.

Related compat issue: mtg-efb050.
