---
title: 'Triskelion: ETB counters not applied, RemoveCounter cost parsed as generic mana'
status: open
priority: 2
issue_type: task
labels:
- single-card
created_at: 2026-04-03T21:29:04.981905091+00:00
updated_at: 2026-04-03T21:29:04.981905091+00:00
---

# Description

Context:
- Date: 2026-04-03
- Puzzle: /tmp/triskelion_test.pzl
- Seed: 42

Two related bugs in Triskelion:

1. ETB +1/+1 counters not placed:
   - Card has K:ETB:Counter<P1P1/3> keyword
   - Engine warns: "Unknown parameterized keyword 'ETB' in 'ETB:Counter<P1P1/3>'"
   - Result: Triskelion enters as 1/1 with 0 counters instead of 4/4 with 3 +1/+1 counters
   - This affects ALL cards with ETB counter keywords (any creature with "enters with N counters")

2. RemoveCounter cost parsed incorrectly:
   - Card has A:AB$ DealDamage | Cost$ RemoveCounter<1/P1P1> | ...
   - Cost parsed as: {generic: 111, red: 1, colorless: 1} (!!!)
   - The "RemoveCounter<1/P1P1>" string is being interpreted as mana cost characters instead of a counter removal cost
   - Fix: Cost::parse() needs to recognize RemoveCounter<N/Type> format

Evidence from snapshot:
- Counters: []  (should be 3 P1P1)
- Ability cost: generic=111  (should be RemoveCounter<1/P1P1>)
- Base power/toughness: 1/1  (correct, but effective should be 4/4)

Expected: Triskelion enters with 3 +1/+1 counters (4/4), can remove a counter to deal 1 damage
Actual: Triskelion enters as 1/1 with no counters, ability costs 111 generic mana

Rules Notes:
- CR 702.19: "Enters the battlefield with N counters" is a replacement effect
- This card's abilities are core to many Old School strategies
