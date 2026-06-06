---
title: 'Card Compatibility: Spider-Sense'
status: open
priority: 3
issue_type: task
created_at: 2026-06-06T04:36:37.155348503+00:00
updated_at: 2026-06-06T04:36:37.155348503+00:00
---

# Description

Test all behavioral aspects of Spider-Sense in MTG Forge-rs.

Card: cardsfolder/s/spider_sense.txt
Set: ATLA (Spider-Man bonus sheet)
Deck: 02 Shibata Izzet Lessons (sideboard)

Card text:
  1U  Instant
  Web-slinging {U} (You may cast this spell for {U} if you also return a tapped creature you control to its owner's hand.)
  Counter target instant spell, sorcery spell, or triggered ability.

Findings (2026-06-05_#3008(50175e06)):

1. [x] Parses: cost 1U, Instant, K:Web-slinging:U
2. [x] Counters spells: "Spider-Sense counters Accumulate Wisdom", "Spider-Sense counters Quantum Riddler"
3. [x] ValidTgts$ Instant,Sorcery,Triggered — correctly targets instant/sorcery spells on the stack
4. [unverified] Web-slinging alternative cost ({U} + return tapped creature)

Reproducer:
```sh
./target/release/mtg tui --p1 zero --p2 zero --p1-draw "Spider-Sense;Island;Island;Island;Island;Island;Island" --p2-draw "Island;Island;Island;Island;Island;Island;Island" --seed 42 --verbosity 3 debug/izzet_sideboard_test.dck debug/izzet_sideboard_test.dck 2>&1 | grep -E "Spider|counters|Counter"
```

Expected:
```
Spider-Sense counters <spell>
```
Actual: Confirmed working.

CARD STATUS: WORKING — counters spells correctly; Web-slinging alternative cost unverified
