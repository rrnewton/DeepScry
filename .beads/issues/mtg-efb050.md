---
title: 'Card Compatibility: Animate Dead'
status: open
priority: 2
issue_type: task
created_at: 2026-05-13T02:20:10.693792605+00:00
updated_at: 2026-05-13T02:44:56.190394872+00:00
---

# Description

PARTIAL FIX 2026-05-12 (compat1).

Set: LEA (mtg-3c7c63)
Deck: rogue_rogerbrand (mtg-526f25)
Card script: cardsfolder/a/animate_dead.txt

Two bugs fixed (now: castable + targeting works):

1. Keyword parser bug — mtg-engine/src/loader/card.rs:
   K:Enchant:Creature.inZoneGraveyard:creature card in a graveyard
   was parsed as Subtype('Creature.inZoneGraveyard:creature card in a graveyard')
   (the second colon was not stripped). Targeting code that split on '.inzone'
   then saw zone='graveyard:creature card in a graveyard' and never matched
   the Some('graveyard') arm.
   Fix: strip everything after the second colon — Subtype is now correctly
   'Creature.inZoneGraveyard'.

2. Aura castability filter bug — mtg-engine/src/game/game_loop/actions.rs:
   The 'is there a valid Aura target?' check at spell-offer time only
   searched the BATTLEFIELD and only matched bare base types ('Creature',
   'Land', etc). For Animate Dead's 'Creature.inZoneGraveyard' the type
   match failed AND the wrong zone was searched.
   Fix: parse '.inZone<X>' qualifier from the Enchant subtype, search zone
   X (currently graveyard) instead of battlefield, and use a
   case-insensitive base-type matcher.

Behavioral aspects verified:
1. [x] Castable for {1}{B} as Sorcery-speed Aura (when a creature card exists in any graveyard)
2. [x] Targets a creature card in any GRAVEYARD (Enchant:Creature.inZoneGraveyard)
3. [x] Aura targeting picks the graveyard creature ('→ targeting Sengir Vampire' logged)
4. [x] Spell resolves on the stack
5. [BROKEN — follow-up needed] ETB trigger T:Mode$ ChangesZone with TrigReanimate: NOT firing. Animate Dead just goes to graveyard after resolution; Sengir Vampire stays in graveyard. The reanimation never happens.
6. [BLOCKED on 5] DBAnimate keyword swap
7. [BLOCKED on 5] DBAttach attaches Animate Dead to the reanimated creature
8. [BLOCKED on 5] -1/-0 continuous effect
9. [BLOCKED on 5] DBDelay sacrifice trigger when Animate Dead leaves
10. [BLOCKED on 5] Cleanup of remembered list

Reproducer:
  ./target/release/mtg tui --start-state test_puzzles/animate_dead_reanimate.pzl --p1=fixed --p2=zero --p1-fixed-inputs='cast Animate Dead' --stop-on-choice=4 --json --seed 42 --verbosity 3

Expected log (current state):
    [1] cast Animate Dead       <-- (was missing before fix)
    Player 1 casts Animate Dead (3) (putting on stack)
    → targeting Sengir Vampire (7)
    Animate Dead (3) resolves
    Animate Dead (3) goes to graveyard   <-- ETB-reanimate trigger not implemented

Regression test: tests/animate_dead_castable_e2e.sh (covers what's fixed).

CARD STATUS: PARTIAL — castable + targeting works, ETB-reanimate trigger pending. Follow-up bug filed.
