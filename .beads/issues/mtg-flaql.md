---
title: 'Card Compatibility: Roaring Furnace / Steaming Sauna'
status: open
priority: 3
issue_type: task
created_at: 2026-06-06T04:31:30.035698773+00:00
updated_at: 2026-06-06T04:31:30.035698773+00:00
---

# Description

Test all behavioral aspects of Roaring Furnace (Room enchantment MDFC) in MTG Forge-rs.

Card: cardsfolder/r/roaring_furnace_steaming_sauna.txt
Set: ATLA / 2025 Standard
Deck: 02 Shibata Izzet Lessons (sideboard)

Card text:
  Roaring Furnace — 1R Enchantment Room
    When you unlock this door, this Room deals damage equal to the number of cards in your hand to target creature an opponent controls.
  Steaming Sauna — 3UU Enchantment Room
    You have no maximum hand size.
    At the beginning of your end step, draw a card.
  (Room MDFC: AlternateMode:Split)

Findings (2026-06-05_#3008(50175e06)):

1. [x] Roaring Furnace half enters as Enchantment
2. [BROKEN] T:Mode$ UnlockDoor trigger not supported (mtg-06mae) — no trigger fires when door unlocked
3. [unverified] Steaming Sauna half (costs 3UU, no maximum hand size + end step draw)
4. [unverified] Unlock door action (paying the mana cost as a sorcery to add the other door's ability)
5. [unverified] AlternateMode:Split parsing / MDFC Room mechanics

Reproducer:
```sh
./target/release/mtg tui --p1 zero --p2 zero --p1-draw "Roaring Furnace;Island;Island;Mountain;Island;Island;Island" --p2-draw "Island;Island;Island;Island;Island;Island;Island" --seed 42 --verbosity 3 debug/izzet_sideboard_test.dck debug/izzet_sideboard_test.dck 2>&1 | grep -E "Roaring|Furnace|Room|door|unlock|Trigger"
```

CARD STATUS: BROKEN — UnlockDoor trigger not supported (mtg-06mae); Room unlock mechanics unimplemented
