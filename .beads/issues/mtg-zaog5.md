---
title: 'Card Compatibility: Giant Growth'
status: closed
priority: 3
issue_type: task
labels:
- puzzle-tested
created_at: 2026-06-14T12:30:29.532797253+00:00
updated_at: 2026-06-14T12:31:43.943348074+00:00
closed_at: 2026-06-14T12:31:43.943348+00:00
---

# Description

Test all behavioral aspects of Giant Growth in MTG Forge-rs.

Set: LEA
Card script: cardsfolder/g/giant_growth.txt
Oracle: Target creature gets +3/+3 until end of turn.
PUZZLE_FILE: test_puzzles/newcard_giant_growth_pump_combat.pzl

Aspects (one per ability/keyword/cost):

1. [x] Card loads as an Instant from cardsfolder (Types: Instant).
2. [x] Castable from hand paying {G} (SP$ Pump spell on the stack).
3. [x] Creature-restricted targeting (ValidTgts$ Creature) resolves.
4. [x] +3/+3 continuous effect applies until end of turn (NumAtt$ +3 / NumDef$ +3); the pumped creature's new P/T is combat-relevant.

Findings (2026-06-14_#3469(2d7639fd1)) - CARD IS WORKING. All 4 aspects verified.

ACTIVE card tested with a [p0_script] that attacks with a 2/2 Grizzly Bears, then (after P1 blocks with its own 2/2) casts Giant Growth on the attacker. The pumped 5/5 kills the 2/2 blocker and survives the 2 damage back. A [p1_script] forces the block so the interaction is deterministic.

Live evidence (mtg tui, scripted, seed 42):
  Player 1 casts Giant Growth (3) (putting on stack)
  Grizzly Bears gets +3/+3 until end of turn
  Grizzly Bears (4) deals 5 damage to Grizzly Bears (11)
  Grizzly Bears (11) dies from combat damage
(P0's attacker survives; assertions: spell cast Giant Growth / opponent graveyard contains Grizzly Bears / creature died Grizzly Bears / me battlefield contains Grizzly Bears all pass.)
