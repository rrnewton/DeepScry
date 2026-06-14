---
title: 'Card Compatibility: Prodigal Sorcerer'
status: closed
priority: 3
issue_type: task
labels:
- puzzle-tested
created_at: 2026-06-14T06:58:25.642787075+00:00
updated_at: 2026-06-14T07:00:48.482503818+00:00
closed_at: 2026-06-14T07:00:48.482503555+00:00
---

# Description

Test all behavioral aspects of Prodigal Sorcerer in MTG Forge-rs.

Set: LEA
Card script: cardsfolder/p/prodigal_sorcerer.txt
Oracle: {T}: Prodigal Sorcerer deals 1 damage to any target.
PT: 1/1
PUZZLE_FILE: test_puzzles/prodigal_sorcerer_ping.pzl

Aspects (one per ability/keyword/cost):

1. [x] Card loads as a 1/1 Creature (Human Wizard Sorcerer) from cardsfolder.
2. [x] Activated ability with a {T} (tap) cost (AB$ DealDamage | Cost$ T) — CR 602.1.
3. [x] Activating taps the Sorcerer as its cost (it ends up tapped).
4. [x] "any target" targeting (ValidTgts$ Any), NumDmg$ 1: deals exactly 1 damage.
5. [x] 1 damage is lethal to a 1/1; the pinged creature dies (CR 704.5g).
6. [x] No summoning sickness for a creature already in play: it can activate the {T} ability the turn the puzzle begins (CR 302.6 applies only to {T} abilities of creatures that came under control this turn).
7. [x] After pinging, the (now untapped next turn) Sorcerer can also attack as a 1/1.

Findings (2026-06-13_#3428(8fc3a787e)) — CARD IS WORKING. All 7 aspects verified.

Live evidence (mtg tui, seed 42):
  <Choice> Player 1 chose 0 - activate Prodigal Sorcerer
  Prodigal Sorcerer activates ability: Prodigal Sorcerer deals 1 damage to any target.
    -> targeting Mons's Goblin Raiders (11)
  Mons's Goblin Raiders (11) takes 1 damage (total: 1)
  Mons's Goblin Raiders (11) dies from lethal damage
  ... Prodigal Sorcerer (5) - 1/1 (tapped)
  Player 1 declares Prodigal Sorcerer (5) (1/1) as attacker
  Prodigal Sorcerer (5) deals 1 damage to Player 2 (life: 19)

Reproducer:
  mtg tui --start-state test_puzzles/prodigal_sorcerer_ping.pzl --p1 heuristic --p2 heuristic --seed 42 -v 2 --no-color-logs

Puzzle assertions (make puzzle-bulk-check): creature died Mons's Goblin Raiders; opponent graveyard contains Mons's Goblin Raiders; life eq 20 — all PASS.

CARD STATUS: WORKING (puzzle-backed).
