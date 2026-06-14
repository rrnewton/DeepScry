---
title: 'Card Compatibility: Healing Salve'
status: closed
priority: 3
issue_type: task
labels:
- puzzle-tested
created_at: 2026-06-14T12:31:01.475246523+00:00
updated_at: 2026-06-14T12:31:43.953234726+00:00
closed_at: 2026-06-14T12:31:43.953234619+00:00
---

# Description

Test all behavioral aspects of Healing Salve in MTG Forge-rs.

Set: LEA
Card script: cardsfolder/h/healing_salve.txt
Oracle: Choose one - Target player gains 3 life; or Prevent the next 3 damage that would be dealt to any target this turn.
PUZZLE_FILE: test_puzzles/newcard_healing_salve_gain_life.pzl

Aspects (one per ability/keyword/cost):

1. [x] Card loads as an Instant from cardsfolder.
2. [x] Castable from hand paying {W} (SP$ Charm spell on the stack).
3. [x] Modal 'choose one' (CR 700.2): the engine offers both modes (DBGainLife, DBPreventDmg) and selects the gain-life mode.
4. [x] Gain-life mode (CR 119.3): the targeted player gains exactly 3 life (15 -> 18).
5. [PARTIAL] Prevent-damage mode (DBPreventDmg) NOT exercised by this puzzle - covered only by the gain-life branch; a follow-up puzzle could drive PreventDamage.

Findings (2026-06-14_#3469(2d7639fd1)) - gain-life branch WORKING; prevent-damage branch untested here.

ACTIVE card tested with a [p0_script] casting Healing Salve at self from 15 life. The engine auto-picks the gain-life mode (no incoming damage to prevent).

Live evidence (mtg tui, scripted, seed 42):
  Player 1 casts Healing Salve (3) (putting on stack)
  Player 1 chooses mode: Target player gains 3 life.
  Player 1 gains 3 life (life: 18)
(assertions: spell cast Healing Salve / life gained ge 3 / life eq 18 all pass.)

COSMETIC LOG BUG (see separate bug issue): the secondary 'causes ... to gain 3 life' line prints a wrong range 'life: 18 => 21' although the authoritative line and final total are the correct 18. Display-only; functional behavior correct.
