---
title: 'Card Compatibility: Torpor Orb'
status: open
priority: 3
issue_type: task
created_at: 2026-06-06T04:36:14.482758009+00:00
updated_at: 2026-06-06T04:36:14.482758009+00:00
---

# Description

Test all behavioral aspects of Torpor Orb in MTG Forge-rs.

Card: cardsfolder/t/torpor_orb.txt
Set: Classic/New Phyrexia
Deck: 04 Henry Temur Otters (mtg-684) — sideboard

Card text:
  {2} Artifact
  Creatures entering don't cause abilities to trigger.

Findings (2026-06-06_#3008(50175e06), agent slot04):

1. [x] Parses as {2} Artifact: WORKING
2. [x] Static ability S:Mode$ DisableTriggers: parses correctly — DisableTriggers mode with ValidCause$ Creature | ValidMode$ ChangesZone,ChangesZoneAll | Destination$ Battlefield
3. [unverified] ETB trigger suppression: Torpor Orb should prevent all creature ETB triggers from firing when it's on the battlefield. Not directly tested in this session (no game run with Torpor Orb in play).
4. [N/A] No ETB trigger on Torpor Orb itself
5. [N/A] AI:RemoveDeck:Random (sideboard-only AI deck hint)

Reproducer:
```sh
./target/release/mtg tui --p1 heuristic --p2 heuristic --seed 42 --verbosity 2 debug/temur_sideboard_test.dck decks/championship/2025/04_henry_temur_otters.dck
```

CARD STATUS: PARTIAL — parses correctly; DisableTriggers static ability not runtime-verified in this session
