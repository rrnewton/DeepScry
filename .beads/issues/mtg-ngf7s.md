---
title: CreatureDied event not emitted for destroy-effect deaths (only lethal-damage)
status: open
priority: 3
issue_type: bug
created_at: 2026-06-14T10:10:41.950540962+00:00
updated_at: 2026-06-14T10:10:41.950540962+00:00
---

# Description

## Observability gap (NOT a card defect)

Found during card-audit wave 2 (mtg-948 Part B) while backing Wrath of God (mtg-558),
Royal Assassin (mtg-537), and Disenchant (mtg-498) with scripted puzzles.

The puzzle-assertion DSL event `creature died [<name>]` checks for a `LogEvent::CreatureDied`
structured event. That event is currently emitted ONLY on lethal-DAMAGE deaths:
- combat damage (mtg-engine/src/game/actions/combat.rs:~1408)
- state-based lethal-damage death (mtg-engine/src/game/state.rs:~3112, 'dies from lethal damage')

A creature that leaves the battlefield via a DESTROY effect (DestroyAll / AB$ Destroy /
SP$ Destroy — Wrath of God, Royal Assassin, Disenchant on creatures, etc.) goes to the
graveyard via a different path that does NOT push a CreatureDied event. The text log DOES
record 'X goes to graveyard' / 'X is destroyed', but no structured CreatureDied event fires.

Per CR 700.4 a creature that is destroyed and moved to the graveyard HAS died, so the event
should fire for destroy-based deaths too (and dies-triggers / death-watchers keying on the
structured event would otherwise miss them).

### Impact
- Puzzle assertions cannot use `creature died` to verify destroy-based removal; they must fall
  back to durable `graveyard contains <name>` final-state checks. The wave-2 Wrath/Royal
  Assassin/Disenchant puzzles do exactly this (see their PUZZLE_FILE notes).
- Any death-trigger logic that consumes the CreatureDied structured event (rather than the
  zone-change) would under-fire on destroy effects. (Combat/burn deaths are fine.)

### Fix direction
Emit LogEvent::CreatureDied at the battlefield->graveyard zone-change boundary for creatures
(the single SBA/zone-move chokepoint), not only on the lethal-damage branch — so all death
causes (damage, destroy, sacrifice, lethal -1/-1, etc.) produce one consistent event. Verify
no double-emission for damage deaths.

### Repro
make puzzle-bulk-check with a puzzle asserting 'creature died <name>' after a Wrath of God /
Royal Assassin destroy — the assertion fails though the creature is in the graveyard.
See test_puzzles/audit_wrath_of_god_board_wipe.pzl (uses graveyard-contains as the workaround).

Filed on branch claude/card-audit-wave2, 2026-06-14_#3463(2a27999fe).
