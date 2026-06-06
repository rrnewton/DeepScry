---
title: 'Card Compatibility: Quantum Riddler'
status: open
priority: 3
issue_type: task
created_at: 2026-06-06T04:35:59.560284230+00:00
updated_at: 2026-06-06T04:35:59.560284230+00:00
---

# Description

Test all behavioral aspects of Quantum Riddler in MTG Forge-rs.

Card: cardsfolder/q/quantum_riddler.txt
Set: ATLA / 2025 Standard
Deck: 01 Manfield Izzet Lessons (sideboard)

Card text:
  3UU  Creature — Sphinx  [4/6]
  Flying
  When this creature enters, draw a card.
  As long as you have one or fewer cards in hand, if you would draw one or more cards, you draw that many cards plus one instead.
  Warp {1}{U}

Findings (2026-06-05_#3008(50175e06)):

1. [x] Parses: cost 3UU, Creature Sphinx 4/6
2. [x] Flying keyword present
3. [unverified] ETB draw trigger (card is expensive, not easily cast in tests)
4. [unverified] Replacement effect: draw extra card when 1 or fewer in hand (R:Event$ DrawCards with SVarCompare$ LE1)
5. [unverified] Warp {1}{U} alternative cost

Reproducer:
```sh
./target/release/mtg tui --p1 zero --p2 zero --p1-draw "Quantum Riddler;Island;Island;Island;Island;Island;Island" --p2-draw "Island;Island;Island;Island;Island;Island;Island" --seed 42 --verbosity 3 debug/izzet_sideboard_test.dck debug/izzet_sideboard_test.dck 2>&1 | grep -E "Quantum|draw|Flying|Warp|warp"
```

CARD STATUS: PARTIAL — parses correctly with Flying; ETB draw, hand-empty draw replacement, and Warp cost unverified
