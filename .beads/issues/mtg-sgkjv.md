---
title: 'Card Compatibility: The Abyss'
status: open
priority: 3
issue_type: task
created_at: 2026-05-28T02:03:18.265620617+00:00
updated_at: 2026-05-28T10:49:55.568619137+00:00
---

# Description

Test all behavioral aspects of The Abyss in MTG Forge-rs for the 1994 Old School playtest goal.

Card file: cardsfolder/t/the_abyss.txt
Set / deck: 1994 Old School playtest; deck 03_robots_jesseisbak (mtg-6ocwz); umbrella mtg-pph0s
Test puzzle: test_puzzles/the_abyss_upkeep_destroy.pzl
Unit test:  test_card_compat_the_abyss + test_destroy_no_regenerate_ignores_shield
            in mtg-engine/src/game/actions/tests/effects.rs
E2E test:   tests/the_abyss_upkeep_destroy_e2e.sh (auto-discovered by shell_script_tests.rs → make validate + CI)

Card text:
  {3}{B} World Enchantment
  At the beginning of each player's upkeep, destroy target nonartifact creature
  that player controls of their choice. It can't be regenerated.

Script:
  T:Mode$ Phase | Phase$ Upkeep | ValidPlayer$ Player | Execute$ TrigDestroy | TriggerZones$ Battlefield
  SVar:TrigDestroy:DB$ Destroy | ValidTgts$ Creature.nonArtifact+ActivePlayerCtrl | NoRegen$ True | TargetingPlayer$ TriggeredPlayer

================================================================
Findings (2026-05-28_#2360(c5681a91), targeted_compatibility agent):
================================================================

Static / shape:
- [x] Parses as {3}{B} World Enchantment (test_card_compat_the_abyss).
- [N/A] P/T: not a creature.
- [N/A] keyword abilities (K:): none printed.

Casting:
- [x] Cast from hand at sorcery speed for {3}{B} (enchantment, standard path; covered by
      the broad casting infrastructure — no card-specific cost logic).
- [N/A] alternative / additional costs, X, hybrid/phyrexian: none.

Triggered ability (T: Mode$ Phase | Phase$ Upkeep | ValidPlayer$ Player):
- [x] Fires at the BEGINNING OF UPKEEP in TriggerZones$ Battlefield (CR 603.2).
- [x] Fires for EACH player's upkeep (ValidPlayer$ Player → NOT controller_turn_only).
      Verified: Turn 1 (P1 active) destroys P1's creature; Turn 2 (P2 active) destroys
      P2's creature. Log:
        Turn 1 - Player 1's turn ... Trigger: The Abyss ... Grizzly Bears (8) goes to graveyard
        Turn 2 - Player 2's turn ... Trigger: The Abyss ... Grizzly Bears (17) goes to graveyard
- [x] Execute$ TrigDestroy resolves a DestroyPermanent (was BROKEN — see root cause below).

Targeting / resolution of the destroy (DB$ Destroy):
- [x] Creature.nonArtifact: artifact creatures are EXCLUDED. Ornithopter (0/2 artifact
      creature) is NOT destroyed; only the nonartifact Grizzly Bears is. (requires_nonartifact)
- [x] ActivePlayerCtrl: only the ACTIVE player's creatures are eligible. On P1's upkeep,
      P2's Grizzly Bears (17) is NOT targeted.
- [PARTIAL] "of their choice": the engine deterministically auto-selects the target
      (lowest CardId among the active player's legal nonartifact creatures) rather than
      asking the active player to choose. This matches the existing project convention for
      triggered abilities (they don't go on the stack) and preserves network determinism /
      controller information-independence. Gameplay outcome is rules-correct (SOME legal
      active-player nonartifact creature dies); only the CHOICE authority differs.
      Follow-up: mtg-9l628.
- [x] NoRegen$ True: destroyed creature can't be regenerated (CR 701.15d). Verified by
      test_destroy_no_regenerate_ignores_shield: a creature with an active regeneration
      shield is destroyed outright when no_regenerate=true, but saved when false.
- [x] No legal target → trigger does nothing (CR 603.10): on later upkeeps with only the
      artifact Ornithopter present, the trigger fires and correctly destroys nothing.

Zone / phase / timing:
- [x] Upkeep phase trigger fires correctly (see above).
- [x] Destroyed creature moves Battlefield → Graveyard.

Interactions:
- [x] Artifact-creature interaction (Ornithopter spared) — the deck 03_robots is artifact-
      heavy, so this is the key intended interaction and it works.
- [unverified] Indestructible target / regeneration via activated ability mid-game:
      no_regenerate path unit-tested directly; in-game activated-regenerate race deferred.

== Root cause that was fixed ==
Two engine gaps made the trigger fire but do nothing:
1. Silent parser drop: the Mode$ Phase trigger's Execute$ handler in
   mtg-engine/src/loader/card.rs had a hardcoded ApiType allowlist
   (DealDamage/GainLife/Earthbend/Pump) that did NOT include Destroy. The Abyss's
   DB$ Destroy produced an empty trigger.effects list. Fixed by reusing the shared
   params_to_effect converter (DRY) for the Destroy ApiType.
2. No target resolution on the phase-trigger path: check_triggers_for_controller
   executed effects directly with the placeholder target unresolved, so DestroyPermanent
   fizzled. Added GameState::choose_triggered_destroy_target (shared with check_triggers)
   to resolve the target among the active player's matching creatures.
Also: TargetRestriction now parses nonArtifact (requires_nonartifact) and ActivePlayerCtrl;
Effect::DestroyPermanent gained a no_regenerate field wired from NoRegen$ True.

== Reproducer ==

```sh
cat > /tmp/the_abyss_upkeep_destroy.pzl <<'P'
[metadata]
Name=The Abyss Upkeep Destroy Test
[state]
turn=2
activeplayer=p0
activephase=UPKEEP
p0life=20
p0battlefield=Swamp;Swamp;Swamp;Swamp;The Abyss;Grizzly Bears;Ornithopter
p0library=Swamp;Swamp;Swamp;Swamp;Swamp
p1life=20
p1battlefield=Forest;Forest;Grizzly Bears
p1library=Forest;Forest;Forest;Forest;Forest
P
./target/release/mtg tui --start-state /tmp/the_abyss_upkeep_destroy.pzl \
  --p1=zero --p2=zero --stop-on-choice=2 --seed 42 --verbosity 3
```

Expected log evidence:

```
Turn 1 - Player 1's turn
--- Upkeep Step ---
  Trigger: The Abyss - At the beginning of each player's upkeep, destroy target nonartifact creature that player controls of their choice. It can't be regenerated.
  Grizzly Bears (8) goes to graveyard
```
(Ornithopter (9), an artifact creature, and Grizzly Bears (17), P2's creature, are NOT destroyed on P1's upkeep.)

CARD STATUS: WORKING — upkeep destroy fires for each player on their upkeep, targets only
the active player's nonartifact creatures, and can't be regenerated. One PARTIAL nuance:
target is engine-auto-chosen rather than player-chosen (mtg-9l628), with rules-correct outcome.
