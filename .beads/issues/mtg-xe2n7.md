---
title: 'Card Compatibility: Roaring Furnace // Steaming Sauna'
status: open
priority: 3
issue_type: task
created_at: 2026-06-06T04:32:01.719875652+00:00
updated_at: 2026-06-06T04:38:07.634259100+00:00
---

# Description

Test all behavioral aspects of Roaring Furnace // Steaming Sauna (Room enchantment) in MTG Forge-rs.

Card: cardsfolder/r/roaring_furnace_steaming_sauna.txt
Set: DSK/Duskmourn: House of Horror
Deck: 04 Henry Temur Otters (mtg-684)

Card text:
  Front face — Roaring Furnace: {1}{R} Enchantment — Room
  When you unlock this door, this Room deals damage equal to the number of cards in your hand to target creature an opponent controls.
  ----
  Back face — Steaming Sauna: {3}{U}{U} Enchantment — Room
  You have no maximum hand size.
  At the beginning of your end step, draw a card.

Findings (2026-06-06_#3008(50175e06), agent slot04):

1. [x] Parses as {1}{R} Enchantment (front face only): WORKING — card loads from deck and shows as '1R' in hand
2. [BROKEN] Back face (Steaming Sauna) not parsed: loader stops at ALTERNATE line. Steaming Sauna face inaccessible. See mtg-u38bu.
3. [BROKEN] ETBReplacement door choice not presented: K:ETBReplacement:Other:SiegeChoice uses DB$ GenericChoice (not DB$ ChoosePlayer). Engine doesn't handle GenericChoice pattern. No door/unlock choice on ETB. See mtg-u38bu.
4. [BROKEN] UnlockDoor trigger (T:Mode$ UnlockDoor) not firing: UnlockDoor mode not implemented in trigger system. Damage trigger cannot fire. See mtg-u38bu.
5. [BROKEN] Heuristic AI never plays card: AI consistently holds Roaring Furnace unplayed across multiple seeds. AI scoring doesn't handle Room enchantments.
6. [x] Zero controller can cast: 'Zero1 casts Frostcliff Siege (18) / resolves' (analogous card test) — card enters battlefield but as a do-nothing enchantment.

Room mechanic bug tracker: mtg-u38bu

Reproducer:
```sh
./target/release/mtg tui --p1 zero --p2 zero --seed 42 --verbosity 3 --p1-draw 'Roaring Furnace;Breeding Pool;Mountain;Island;Breeding Pool' --p2-draw 'Island;Island;Island;Island;Island;Island;Island' debug/frostcliff_test.dck debug/frostcliff_test.dck
```

Expected (BROKEN — no door choice, no trigger):
```
Zero1 casts Roaring Furnace (N) (putting on stack)
Roaring Furnace (N) resolves
```
(No 'choose Roaring Furnace or Steaming Sauna' prompt; no damage trigger)

CARD STATUS: BROKEN — Room mechanic not implemented; card enters without mode choice and no triggers fire
