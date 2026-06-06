---
title: 'Card Compatibility: Ghost Vacuum'
status: open
priority: 3
issue_type: task
created_at: 2026-06-06T04:35:39.694695866+00:00
updated_at: 2026-06-06T04:35:39.694695866+00:00
---

# Description

Test all behavioral aspects of Ghost Vacuum in MTG Forge-rs.

Card: cardsfolder/g/ghost_vacuum.txt
Set: ATLA / 2025 Standard
Deck: 02 Shibata Izzet Lessons (sideboard)

Card text:
  1  Artifact
  {T}: Exile target card from a graveyard.
  {6}, {T}, Sacrifice Ghost Vacuum: Put each creature card exiled with Ghost Vacuum onto the battlefield under your control with a flying counter on it. Each of them is a 1/1 Spirit in addition to its other types. Activate only as a sorcery.

Findings (2026-06-05_#3008(50175e06)):

1. [x] Parses: cost 1, Artifact
2. [x] Enters battlefield
3. [x] Second ability (sacrifice) fires in game: "Ghost Vacuum activates ability: Put each creature card exiled with Ghost Vacuum onto the battlefield..." and "Ghost Vacuum (27) goes to graveyard"
4. [unverified] First {T} exile ability targeting graveyard cards
5. [unverified] Creatures entering with flying counter
6. [unverified] ExiledWithSource tracking (needed for second ability to know which cards to reanimate)

Reproducer:
```sh
./target/release/mtg tui --p1 zero --p2 zero --p1-draw "Ghost Vacuum;Island;Island;Island;Island;Island;Island" --p2-draw "Island;Island;Island;Island;Island;Island;Island" --seed 42 --verbosity 3 debug/izzet_sideboard_test.dck debug/izzet_sideboard_test.dck 2>&1 | grep -E "Ghost|Vacuum|exile|spirit|counter|flying"
```

CARD STATUS: PARTIAL — parses, enters, sacrifice ability fires; exile ability and creature reanimation unverified
