---
title: 'Card Compatibility: Broadside Barrage'
status: open
priority: 3
issue_type: task
created_at: 2026-06-06T04:35:49.229317543+00:00
updated_at: 2026-06-06T04:35:49.229317543+00:00
---

# Description

Test all behavioral aspects of Broadside Barrage in MTG Forge-rs.

Card: cardsfolder/b/broadside_barrage.txt
Set: ATLA / 2025 Standard
Deck: 01 Manfield Izzet Lessons (sideboard)

Card text:
  1UR  Instant
  Broadside Barrage deals 5 damage to target creature or planeswalker. Draw a card, then discard a card.

Findings (2026-06-05_#3008(50175e06)):

1. [x] Parses: cost 1UR, Instant
2. [x] Resolves: deals 5 damage, draws a card, discards a card
3. [x] Log: "Broadside Barrage deals 5 damage to Zero2" (targeting player when no creatures exist — targeting bypass issue, should be creature or planeswalker only)

Reproducer:
```sh
./target/release/mtg tui --p1 zero --p2 zero --p1-draw "Broadside Barrage;Island;Island;Mountain;Island;Island;Island" --p2-draw "Island;Island;Island;Island;Island;Island;Island" --seed 42 --verbosity 3 debug/izzet_sideboard_test.dck debug/izzet_sideboard_test.dck 2>&1 | grep -E "Broadside|deals|draw|discard"
```

Expected: "Broadside Barrage deals 5 damage to <creature>"
Actual: "Broadside Barrage deals 5 damage to Zero2" (5 damage to player — wrong targeting, but damage amount is correct)

CARD STATUS: PARTIAL — damage amount correct (5); draws and discards correctly; targeting bypass (hits player instead of creature/planeswalker when no legal targets)
