---
title: 'Event log: TriggerFired not emitted for DamageDone / Phase(upkeep) triggers'
status: open
priority: 3
issue_type: bug
labels:
- puzzle-gap
created_at: 2026-06-14T07:36:05.410328669+00:00
updated_at: 2026-06-14T07:36:05.410328669+00:00
---

# Description

Found during mtg-948 Part B card-audit puzzle backfill (claude/card-audit-wave1).

## Summary
The puzzle assertion DSL predicate \`trigger fired from <CardName>\` (LogEvent::TriggerFired) is only emitted by the engine for a NARROW set of death/leaves-battlefield watcher triggers. It is NOT emitted for other trigger kinds that DO fire correctly in-engine, so puzzles cannot assert those triggers via the event log even though the underlying card behavior is correct.

## Where TriggerFired is emitted today
mtg-engine/src/game/actions/mod.rs emits LogEvent::TriggerFired only for:
- TriggerEvent::LeavesBattlefield (~line 7988)
- TriggerEvent::EquippedCreatureDies (~line 8079)
- TriggerEvent::DamagedCreatureDies (~line 8144)
- a death-watcher "a creature dies" trigger (~line 8231; Fecundity-style)

## Trigger kinds that fire but emit NO TriggerFired event
- DamageDone triggers (e.g. Hypnotic Specter "Whenever CARDNAME deals damage to an opponent, that player discards a card at random"). The discard DOES happen (verified in game log: "Player 2 discards Forest"), but no TriggerFired event is recorded.
- Phase / beginning-of-upkeep triggers (e.g. Serendib Efreet "At the beginning of your upkeep, CARDNAME deals 1 damage to you"). The self-damage DOES happen (game log: "Serendib Efreet deals 1 damage to Player 1"), but no TriggerFired event is recorded.

## Reproduction
make puzzle-bulk-check with a puzzle containing \`trigger fired from Hypnotic Specter\` or \`trigger fired from Serendib Efreet\` -> ASSERT_FAIL "predicate evaluated to false", despite the discard / self-damage visibly occurring in -v 2 logs.

Reproducer commands:
  mtg tui --start-state test_puzzles/audit_hypnotic_specter_discard.pzl --p1 heuristic --p2 heuristic --seed 42 -v 2 --no-color-logs   # shows "Player 2 discards Forest"
  mtg tui --start-state test_puzzles/audit_serendib_efreet_upkeep_selfdamage.pzl --p1 heuristic --p2 heuristic --seed 42 -v 2 --no-color-logs   # shows "Serendib Efreet deals 1 damage to Player 1"

## Impact / current workaround
Card behavior is CORRECT; this is purely an event-log instrumentation gap. The two audit puzzles above were written to assert the OBSERVABLE effect instead (opponent graveyard count ge 1 for the discard; me life lt 20 for the upkeep self-damage), so the cards are still puzzle-backed (mtg-510 Hypnotic Specter, mtg-540 Serendib Efreet). This issue tracks broadening TriggerFired emission so puzzles can assert these trigger kinds directly.

## Suggested fix
Emit LogEvent::TriggerFired at the central trigger-resolution point (where a triggered ability is put on the stack / resolves) for ALL TriggerEvent kinds, rather than only at the per-event death/leave sites. Tracking issue mtg-935 (puzzle assertion DSL) and mtg-947 (event-log wiring) are related.

Relationship to Java Forge: Forge-java fires a generic Trigger object for every trigger kind via the TriggerHandler; this Rust port currently only surfaces a subset to its structured event log. The card semantics match; only the diagnostic event stream differs.
