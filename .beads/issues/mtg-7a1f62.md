---
title: 'Card Compatibility: Sengir Vampire'
status: open
priority: 3
issue_type: task
created_at: 2026-05-13T03:00:01.049748132+00:00
updated_at: 2026-05-13T03:00:01.049748132+00:00
---

# Description

Test all behavioral aspects of Sengir Vampire in MTG Forge-rs.

Card: cardsfolder/s/sengir_vampire.txt
Set: LEA (Alpha)

Card text:
  Sengir Vampire {3}{B}{B}, 4/4 Creature - Vampire
  Flying
  Whenever a creature dealt damage by Sengir Vampire this turn dies,
  put a +1/+1 counter on Sengir Vampire.

Findings (2026-05-12, compat2):

1. [x] Parses as 4/4 Creature - Vampire, cost {3}{B}{B}
2. [x] Has Keyword::Flying on the parsed card
3. [BROKEN] DamagedBy trigger silently dropped:
   - The trigger is
     'T:Mode\$ ChangesZone | Origin\$ Battlefield | Destination\$ Graveyard
                          | ValidCard\$ Creature.DamagedBy
                          | TriggerZones\$ Battlefield | Execute\$ TrigPutCounter'
   - The ChangesZone trigger parser at loader/card.rs:1874 only matches
     'ValidCard\$ Card.Self' (line 1877). Line 1901 matches
     'Card.EquippedBy' for Skullclamp-style triggers. There is no
     branch matching 'Creature.DamagedBy' (or any of the other
     'Creature.X' patterns), so this trigger never gets registered.
   - Even with parser support, the engine does not currently track
     'which sources have damaged which creatures this turn', so the
     condition could not be evaluated. Affects all 'Whenever a creature
     dealt damage by ~ this turn dies' effects.

Filed as separate bug: mtg-c-bug-damaged-by-trigger

Reproducer (would show the bug if DamagedBy worked): cast Sengir
Vampire, attack into a creature it can kill (deal 4 damage, target
dies), Sengir Vampire should get +1/+1 counter and become 5/5.
Currently: Sengir Vampire stays 4/4, no counter is added.

Unit test: test_card_compat_sengir_vampire in mtg-engine/src/game/actions/tests/effects.rs
(asserts the static side: 4/4 Vampire with Flying. Trigger gap is
documented in the bug above.)

CARD STATUS: PARTIAL — vanilla 4/4 flier works; the conditional
counter-on-kill trigger is silently dropped. Card is functional but
strictly weaker than its printed text.
