---
title: 'Bug: Room mechanic (Duskmourn) not implemented — UnlockDoor trigger, door state, split enchantment casting'
status: open
priority: 3
issue_type: task
created_at: 2026-06-06T04:37:48.287907843+00:00
updated_at: 2026-06-06T04:37:48.287907843+00:00
---

# Description

Room cards from Duskmourn: House of Horror (DSK) are a new enchantment subtype with a split-door mechanic. The mechanic is not implemented in the engine.

Missing features:
1. AlternateMode:Split for Rooms: The card file uses ALTERNATE separator to define two halves (doors). The loader stops at ALTERNATE and only reads the front face. The back face (e.g., Steaming Sauna) is inaccessible.
2. ETBReplacement via DB$ GenericChoice: Room cards use K:ETBReplacement:Other:SiegeChoice where SVar:SiegeChoice:DB$ GenericChoice | Choices$ A,B. The engine only handles ChooseColor and ChoosePlayer ETBReplacement patterns; GenericChoice is not recognized. The door-unlock choice is never presented.
3. UnlockDoor trigger mode (T:Mode$ UnlockDoor): No handling in the trigger system. 'When you unlock this door' triggers cannot fire.
4. Door state tracking: locked vs unlocked state per half of a Room not tracked in GameState.
5. Separate casting of each half: In MTG rules, you may cast either half of a Room card and it enters as that door unlocked.

Affected cards (confirmed):
- Roaring Furnace // Steaming Sauna (mtg-xe2n7) — enters as plain 1R enchantment, no door choice, no triggers

Affected cards (likely — DSK set rooms):
- All Room cards from Duskmourn

Example card: Roaring Furnace // Steaming Sauna
- Front: {1}{R} Room — When you unlock this door, deal damage to target creature equal to cards in hand
- Back: {3}{U}{U} Room — No hand size limit + draw at end step

Reproducer:
```sh
./target/release/mtg tui --p1 zero --p2 zero --seed 42 --verbosity 3 --p1-draw 'Roaring Furnace;Breeding Pool;Mountain;Island;Breeding Pool' --p2-draw 'Island;Island;Island;Island;Island;Island;Island' debug/frostcliff_test.dck debug/frostcliff_test.dck
```

Expected: mode-choice prompt 'Roaring Furnace' or 'Steaming Sauna' on ETB.
Actual: card enters with no choice, no triggers.
