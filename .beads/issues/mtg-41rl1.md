---
title: 'Card Compatibility: Shock'
status: closed
priority: 3
issue_type: task
labels:
- puzzle-tested
created_at: 2026-06-14T06:58:11.188124045+00:00
updated_at: 2026-06-14T07:00:48.478703033+00:00
closed_at: 2026-06-14T07:00:48.478702613+00:00
---

# Description

Test all behavioral aspects of Shock in MTG Forge-rs.

Set: LEA
Card script: cardsfolder/s/shock.txt
Oracle: Shock deals 2 damage to any target.
PUZZLE_FILE: test_puzzles/shock_any_target_two_damage.pzl

Aspects (one per ability/keyword/cost):

1. [x] Card loads as an Instant from cardsfolder (Types: Instant).
2. [x] Castable from hand paying {R} (SP$ DealDamage spell on the stack).
3. [x] "any target" targeting (ValidTgts$ Any) — resolves to a legal target (creature).
4. [x] NumDmg$ 2: deals exactly 2 damage to the target.
5. [x] Lethal-damage state-based action: a 2/2 (Grizzly Bears) with 2 marked damage is destroyed (CR 120.6 / 704.5g).
6. [x] After resolving, Shock moves to its controller's graveyard (CR 608.2m).

Findings (2026-06-13_#3428(8fc3a787e)) — CARD IS WORKING. All 6 aspects verified.

Live evidence (mtg tui, seed 42):
  <Choice> Player 1 chose 0 - cast Shock
  Player 1 casts Shock (3) (putting on stack)
    -> targeting Grizzly Bears (10)
  Shock (3) resolves
  Grizzly Bears (10) takes 2 damage (total: 2)
  Shock (3) deals 2 damage to Grizzly Bears (10)
  Grizzly Bears (10) dies from lethal damage

Reproducer:
  mtg tui --start-state test_puzzles/shock_any_target_two_damage.pzl --p1 heuristic --p2 heuristic --seed 42 -v 2 --no-color-logs

Puzzle assertions (make puzzle-bulk-check): spell cast Shock; creature died Grizzly Bears; me graveyard contains Shock; opponent graveyard contains Grizzly Bears — all PASS.

CARD STATUS: WORKING (puzzle-backed).
