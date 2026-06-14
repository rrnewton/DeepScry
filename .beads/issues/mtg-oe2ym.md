---
title: 'Card Compatibility: Llanowar Elves'
status: closed
priority: 3
issue_type: task
labels:
- puzzle-tested
created_at: 2026-06-14T12:30:43.206146912+00:00
updated_at: 2026-06-14T12:31:43.946106156+00:00
closed_at: 2026-06-14T12:31:43.946106060+00:00
---

# Description

Test all behavioral aspects of Llanowar Elves in MTG Forge-rs.

Set: LEA
Card script: cardsfolder/l/llanowar_elves.txt
Oracle: {T}: Add {G}.
PUZZLE_FILE: test_puzzles/newcard_llanowar_elves_mana.pzl

Aspects (one per ability/keyword/cost):

1. [x] Card loads as a 1/1 Elf Druid creature from cardsfolder.
2. [x] Activated mana ability parses (AB$ Mana, Cost$ T, Produced$ G; CR 605).
3. [x] The {T}: Add {G} ability produces green mana that is USABLE to pay a cost - proven by casting a {1}{G} Grizzly Bears off a single Forest plus the Elves (one land alone cannot pay {1}{G}).

Findings (2026-06-14_#3469(2d7639fd1)) - CARD IS WORKING. All 3 aspects verified.

ACTIVE card tested with a [p0_script] casting a {1}{G} Grizzly Bears when P0 controls one Forest + the Llanowar Elves. The successful cast PROVES the Elves contributed mana (one Forest only makes {G}; the {1} generic must come from the Elves).

Live evidence (mtg tui, scripted, seed 42):
  Player 1 casts Grizzly Bears (3) (putting on stack)
  Tap Llanowar Elves for mana
  Grizzly Bears (3) resolves
  Grizzly Bears (3) enters the battlefield as a 2/2 creature
(assertions: spell cast Grizzly Bears / me battlefield contains Grizzly Bears / me battlefield contains Llanowar Elves all pass.)
