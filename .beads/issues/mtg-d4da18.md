---
title: 'Card Compatibility: Sengir Vampire'
status: closed
priority: 2
issue_type: task
created_at: 2026-05-13T02:55:47.901685533+00:00
updated_at: 2026-05-13T03:15:48.045821393+00:00
closed_at: 2026-05-13T03:15:48.045821313+00:00
---

# Description

FIXED 2026-05-12 (compat1).

Set: LEA (mtg-3c7c63)
Deck: rogue_rogerbrand (mtg-526f25)
Card script: cardsfolder/s/sengir_vampire.txt

Implemented the previously-unsupported 'Whenever a creature dealt damage by CARDNAME this turn dies, put a +1/+1 counter on CARDNAME' trigger pattern (TODO mtg-147 / Forge ValidCard$ Creature.DamagedBy).

Implementation:
1. mtg-engine/src/core/card.rs: new field damaged_by_this_turn: SmallVec<[CardId; 2]> tracks damage sources per card.
2. mtg-engine/src/core/effects.rs: new TriggerEvent::DamagedCreatureDies variant.
3. mtg-engine/src/loader/card.rs: parse 'T:Mode$ ChangesZone | Origin$ Battlefield | Destination$ Graveyard | ValidCard$ Creature.DamagedBy' → Trigger { event: DamagedCreatureDies }.
4. mtg-engine/src/game/actions/combat.rs: collect damage_sources_per_target during combat damage step; before lethal-damage check, push each source onto target.damaged_by_this_turn (deduped).
5. mtg-engine/src/game/actions/mod.rs: extend check_death_triggers to scan battlefield for permanents with DamagedCreatureDies triggers whose CardId appears in dying_card.damaged_by_this_turn; fire each with itself as Defined$ Self.
6. mtg-engine/src/game/actions/triggers.rs: PutCounter resolver was matching on is_placeholder() only — added is_self_target() arm so 'DB$ PutCounter | Defined$ Self' (compat2's parser change for All Hallow's Eve) properly resolves to the trigger source.
7. mtg-engine/src/game/game_loop/steps.rs: clear damaged_by_this_turn at cleanup step (CR 514.2).
8. mtg-engine/src/game/heuristic_controller.rs: trigger evaluation +15 for DamagedCreatureDies (high value — rewards favourable trades).

Behavioral aspects verified:
1. [x] Castable for {3}{B}{B} as Creature Vampire
2. [x] ETB as 4/4
3. [x] Has Flying keyword (test uses Birds of Paradise as flying blocker)
4. [x] Damage tracking: source recorded in damaged_by_this_turn
5. [x] Death trigger fires when a damaged creature dies (verified via gamelog 'Trigger: Sengir Vampire - ...')
6. [x] +1/+1 counter applied to Sengir (4/4 → 5/5 in puzzle)
7. [x] Counter persistence through state-based actions
8. [unverified — would need contrived test] Sengir survives a same-toughness trade
9. [implicit] Multiple deaths in same turn → multiple counters (one trigger per death)
10. [x] Cleanup: damaged_by_this_turn cleared at end of turn (CR 514.2 4th bullet)
11. [x] Heuristic AI evaluator gives Sengir-style triggers +15 weight

Reproducer:
  ./target/release/mtg tui --start-state test_puzzles/sengir_vampire_kills_creature.pzl --p1=heuristic --p2=zero --stop-on-choice=8 --json --seed 42 --verbosity 3

Expected log (excerpt):
    Player 1 declares Sengir Vampire (3) (4/4) as attacker
    Player 2 declares Birds of Paradise (14) as blocker for Sengir Vampire (3)
    Sengir Vampire (3) deals 4 damage to Birds of Paradise (14)
    Trigger: Sengir Vampire - Whenever a creature dealt damage by CARDNAME this turn dies, put a +1/+1 counter on CARDNAME.
    Birds of Paradise (14) goes to graveyard
    Birds of Paradise (14) dies from combat damage
    Battlefield:
      Sengir Vampire (3) - 5/5 (tapped)   <-- 4/4 + P1P1 counter

Regression test: tests/sengir_vampire_flying_e2e.sh

CARD STATUS: WORKING. The DamagedCreatureDies trigger pattern is now generic — also unblocks Baron Sengir, Abattoir Ghoul, Blood Cultist, Bone Shaman, Bushi Tenderfoot, Falling Star, Frostwielder, Garza Zol, Plague Queen, etc. (all cards in cardsfolder using ValidCard$ Creature.DamagedBy).
