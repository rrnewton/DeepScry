---
title: 'Card Compatibility: Elvish Archers'
status: closed
priority: 3
issue_type: task
labels:
- puzzle-tested
created_at: 2026-06-14T12:30:43.208961184+00:00
updated_at: 2026-06-14T12:31:43.948414465+00:00
closed_at: 2026-06-14T12:31:43.948414401+00:00
---

# Description

Test all behavioral aspects of Elvish Archers in MTG Forge-rs.

Set: LEA
Card script: cardsfolder/e/elvish_archers.txt
Oracle: First strike
PUZZLE_FILE: test_puzzles/newcard_elvish_archers_first_strike.pzl

Aspects (one per ability/keyword/cost):

1. [x] Card loads as a 2/1 Elf Archer creature from cardsfolder.
2. [x] K:First Strike parses and is applied.
3. [x] First-strike combat ordering (CR 702.7): a 2/1 first striker blocked by a 2/2 Grizzly Bears deals its 2 damage FIRST, killing the blocker before it can deal damage back, so the 1-toughness Archers survives unscathed - an outcome impossible without first strike (simultaneous damage would kill a 2/1).

Findings (2026-06-14_#3469(2d7639fd1)) - CARD IS WORKING. All 3 aspects verified.

PASSIVE keyword tested with a [p0_script] attack + [p1_script] block. The survival of the 2/1 Archers is the proof that first strike is enforced.

Live evidence (mtg tui, scripted, seed 42):
  --- First Strike Combat Damage ---
  Elvish Archers (3) deals 2 damage to Grizzly Bears (10)
  Grizzly Bears (10) dies from combat damage
  --- Normal Combat Damage ---
(assertions: opponent graveyard contains Grizzly Bears / creature died Grizzly Bears / me battlefield contains Elvish Archers / life eq 20 all pass.)
