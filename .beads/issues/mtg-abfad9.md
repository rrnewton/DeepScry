---
title: 'Bug: Animate Dead ETB-reanimate trigger not firing'
status: open
priority: 3
issue_type: bug
created_at: 2026-05-13T02:45:16.381440871+00:00
updated_at: 2026-05-13T02:45:16.381440871+00:00
---

# Description

Animate Dead's ETB self-trigger T:Mode$ ChangesZone | Origin$ Any | Destination$ Battlefield | ValidCard$ Card.Self | IsPresent$ Card.StrictlySelf | Execute$ TrigReanimate is not firing during the spell's resolution. As a result the targeted creature is never returned from graveyard to battlefield; Animate Dead simply moves itself to the graveyard.

After the casting/targeting fixes (mtg-efb050), Animate Dead resolves on the stack and the player chooses a creature in graveyard, but the ETB chain is dropped:

  Player 1 casts Animate Dead (3) (putting on stack)
  → targeting Sengir Vampire (7)
  Animate Dead (3) resolves
  Animate Dead (3) goes to graveyard
  (Sengir Vampire stays in graveyard; battlefield empty)

Required SubAbility chain (from cardsfolder/a/animate_dead.txt):
  TrigReanimate: DB$ ChangeZone | Origin$ Graveyard | Destination$ Battlefield | Defined$ Enchanted | RememberChanged$ True | GainControl$ True | SubAbility$ DBAnimate
  DBAnimate: DB$ Animate | Defined$ Self | Keywords$ Enchant:Creature.IsRemembered:... | RemoveKeywords$ Enchant:Creature.inZoneGraveyard:... | SubAbility$ DBAttach
  DBAttach: DB$ Attach | Defined$ Remembered | SubAbility$ DBDelay
  DBDelay: DB$ DelayedTrigger | Mode$ ChangesZone | ValidCard$ Card.Self | Origin$ Battlefield | Execute$ TrigSacrifice | RememberObjects$ RememberedLKI

Implementation gaps to investigate:
1. Is the ETB-self trigger T:Mode$ ChangesZone | ValidCard$ Card.Self being parsed at all on Aura cards?
2. Does the engine recognize Defined$ Enchanted to mean 'the targeted card I'm about to enchant'?
3. Is there a code path that handles ChangeZone Origin$ Graveyard | Destination$ Battlefield with GainControl$ True?
4. Animate effect (keyword swap) and DBDelay (delayed trigger registration) likely need new handlers too.

Related compat issue: mtg-efb050. The cast/targeting parts are fixed; this issue tracks the much-larger reanimation-trigger work. Affects: Animate Dead, Dance of the Dead, Spellweaver Volute, Reanimate (similar SP$ ChangeZone Graveyard→Battlefield).
