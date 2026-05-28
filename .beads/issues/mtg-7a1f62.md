---
title: 'Card Compatibility: Sengir Vampire'
status: closed
priority: 3
issue_type: task
created_at: 2026-05-13T03:00:01.049748132+00:00
updated_at: 2026-05-28T11:47:22.438243638+00:00
closed_at: 2026-05-28T11:47:22.438243587+00:00
---

# Description

Test all behavioral aspects of Sengir Vampire in MTG Forge-rs.

Card: cardsfolder/s/sengir_vampire.txt
Set: LEA (Alpha)
Goal: 1994 Old School playtest (mtg-pph0s)

Card text:
  Sengir Vampire {3}{B}{B}, 4/4 Creature - Vampire
  Flying
  Whenever a creature dealt damage by Sengir Vampire this turn dies,
  put a +1/+1 counter on Sengir Vampire.

Findings (2026-05-12, compat2): the DamagedBy trigger was silently dropped
by the ChangesZone parser. FIXED in sibling issue mtg-d4da18 (compat1):
parser branch added, TriggerEvent::DamagedCreatureDies + per-card
damaged_by_this_turn tracking implemented, cleared at cleanup (CR 514.2).

Findings (2026-05-28_#2360(897881c9), verify-sengir-vampire) — RE-VERIFIED:

1. [x] Static/shape: parses as 4/4 Creature - Vampire, cost {3}{B}{B}, black.
       Asserted in test_card_compat_sengir_vampire (cost.generic=3,
       cost.black=2, power=4, toughness=4, type Creature).
2. [x] Flying keyword parses AND registers. Unit test asserts
       card.keywords.contains(Keyword::Flying). Honored in combat: the e2e
       puzzle uses Birds of Paradise (0/1 FLYING) as the only legal blocker
       for Sengir — a ground creature could not block (CR 702.9).
3. [x] Trigger parses (regression guard): unit test now asserts the card
       registers a DamagedCreatureDies trigger whose effect is
       PutCounter{P1P1}. This pins the previously-dropped trigger so the
       silent-drop cannot regress.
4. [x] Trigger fires on COMBAT death -> +1/+1 counter (Sengir 4/4 -> 5/5).
       Verified via game log (tests/sengir_vampire_flying_e2e.sh):
         Sengir Vampire (3) deals 4 damage to Birds of Paradise (14)
         Trigger: Sengir Vampire - Whenever a creature dealt damage by ...
         Birds of Paradise (14) dies from combat damage
         Sengir Vampire (3) - 5/5 (tapped)
5. [x] "This turn" linkage (damaged now, dies LATER from a non-combat
       source) -> still triggers. Verified by
       test_card_compat_sengir_vampire_this_turn_linkage: simulates the
       combat-recorded damaged_by_this_turn state (combat.rs records the
       source BEFORE the lethal check, for ALL combat damage incl.
       sublethal), then kills the victim via the shared check_death_triggers
       death path (destroy/sacrifice/SBA) -> Sengir gains a P1P1 counter.
6. [x] Does NOT trigger for creatures Sengir did not damage. Verified two
       ways: (a) gameplay — two Grizzly Bears trade in a combat Sengir was
       not part of; no "Trigger: Sengir Vampire" line fires and Sengir stays
       4/4; (b) unit test test_card_compat_sengir_vampire_no_trigger_when_undamaged
       (empty damaged_by_this_turn -> 0 counters).
7. [N/A] Casting/alt-costs/activated/replacement: vanilla creature, no
       alternative costs, no activated/static/replacement abilities.

Verification-only: NO engine code changed. The trigger, damage tracking,
cleanup, and counter application were already correct (mtg-d4da18). This
pass adds regression tests and refreshes stale tracking that still read
PARTIAL/BROKEN.

MTG rules compliance (no code change, noted per skill): CR 702.9 (Flying —
only flyers/reach block), CR 603.2/603.6 (death trigger timing), CR 120
(combat damage), CR 122 (+1/+1 counters), CR 514.2 (damage/damaged-by reset
at cleanup) all honored.

Reproducer (combat-death trigger, runs in make validate as
shell_scripts__sengir_vampire_flying_e2e):

```sh
./target/release/mtg tui \
  --start-state test_puzzles/sengir_vampire_kills_creature.pzl \
  --p1=heuristic --p2=zero --stop-on-choice=8 --json --seed 42 --verbosity 3
```

Expected log evidence:

```
Sengir Vampire (3) deals 4 damage to Birds of Paradise (14)
Trigger: Sengir Vampire - Whenever a creature dealt damage by CARDNAME this turn dies, put a +1/+1 counter on CARDNAME.
Birds of Paradise (14) dies from combat damage
Sengir Vampire (3) - 5/5 (tapped)
```

Unit tests (mtg-engine/src/game/actions/tests/effects.rs):
  test_card_compat_sengir_vampire                 (shape + Flying + trigger parse)
  test_card_compat_sengir_vampire_this_turn_linkage   (non-combat later death)
  test_card_compat_sengir_vampire_no_trigger_when_undamaged (negative case)
E2E test: tests/sengir_vampire_flying_e2e.sh (auto-discovered via dir-test)

CARD STATUS: WORKING — 4/4 flier, DamagedCreatureDies +1/+1 trigger fires on
combat and non-combat deaths of Sengir-damaged creatures, and correctly does
not fire otherwise.
