---
title: 'Card Compatibility: Stone Rain'
status: closed
priority: 3
issue_type: task
labels:
- puzzle-tested
created_at: 2026-06-14T10:09:50.444410437+00:00
updated_at: 2026-06-14T10:10:19.135268587+00:00
closed_at: 2026-06-14T10:10:19.135268513+00:00
---

# Description

Test all behavioral aspects of Stone Rain in MTG Forge-rs.

Set: LEA
Card script: cardsfolder/s/stone_rain.txt
Oracle: Destroy target land.
PUZZLE_FILE: test_puzzles/script_stone_rain_destroys_land.pzl

Aspects (one per ability/keyword/cost):

1. [x] Card loads as a Sorcery from cardsfolder (Types: Sorcery).
2. [x] Castable from hand paying {2}{R} (SP$ Destroy spell on the stack).
3. [x] Land-restricted targeting (ValidTgts$ Land) resolves to a land.
4. [x] Destroy effect moves the targeted land battlefield -> owner graveyard (CR 701.7 / 704).

Findings (2026-06-14_#3463(2a27999fe)) - CARD IS WORKING. All 4 aspects verified.

ACTIVE card tested with a puzzle ACTION SCRIPT ([p0_script]) forcing P0 to cast it at P1's Forest. The opponent's land is a FOREST (unique name on the board) so the name selector unambiguously picks it rather than one of P0's own Mountains.

Live evidence (mtg tui, scripted, seed 42):
  Player 1 casts Stone Rain (3) (putting on stack)
    -> targeting Forest (12)
  Stone Rain (3) resolves
  Stone Rain (3) destroys Forest (12)

Reproducer:
  mtg tui --start-state test_puzzles/script_stone_rain_destroys_land.pzl --p1 fixed --p1-fixed-inputs "cast Stone Rain targeting Forest" --p2 heuristic --seed 42 -v 2 --no-color-logs

Puzzle assertions (make puzzle-bulk-check): spell cast Stone Rain; opponent graveyard contains Forest - all PASS. (No end-of-game battlefield-count check: assertions evaluate on the final state, by which point the opponent has replayed lands.)

CARD STATUS: WORKING (puzzle-backed).
