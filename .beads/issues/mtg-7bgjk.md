---
title: 'Puzzle DSL Phase 2: structured game-log events (LogEvent enum) for trigger/creature-death/spell-cast assertions'
status: closed
priority: 3
issue_type: task
created_at: 2026-06-13T23:00:36.509627717+00:00
updated_at: 2026-06-13T23:56:57.043735358+00:00
closed_at: 2026-06-13T23:56:57.043735306+00:00
---

# Description

## Purpose
Phase 2 of the puzzle assertion DSL. Unblocks event-level assertions (trigger fired, creature died, spell cast) by providing a strongly-typed structured log alongside the existing string log.

## Background
Phase 1 (mtg-0oopj, landed) implements final-state assertions only. Event assertions were deferred because the string log violates the NO HACKY STRING OPERATIONS rule. This issue tracks the structured log foundation that enables them.

## Implementation (fully landed on claude/puzzle-trigger-assertions)

### Core files
- mtg-engine/src/game/log_event.rs — LogEvent enum (SpellCast, TriggerFired, CreatureDied, ZoneChange, DamageDealt, LifeChanged, TurnStarted, StepStarted)
- mtg-engine/src/game/logger.rs — GameLogger gains event_log: Vec<LogEvent> + enable_event_log_enabled: bool
- mtg-engine/src/game/actions/effects/life.rs — LifeChanged events wired in execute_gain_life, execute_gain_life_dynamic, execute_lose_life
- mtg-engine/src/puzzle/assert/mod.rs — AssertionKind variants: TriggerFired, SpellCast, CreatureDied, LifeGained
- mtg-engine/src/puzzle/assert/parser.rs — Parses: 'trigger fired [from <name>]', 'spell cast [<name>]', 'creature died [<name>]', 'life gained <cmp> <N>'
- mtg-engine/src/puzzle/assert/evaluator.rs — evaluate_assertions gains events: Option<&EventLogView<'_>> param; new arms for all 4 event kinds
- mtg-engine/tests/puzzle_assert_e2e.rs — run_puzzle enables event log; passes Some(&events) to evaluate_assertions
- mtg-engine/tests/puzzle_e2e.rs — Spirit Link tests #11/#12/#13 migrated from string-log to LifeChanged event checks

### Zero-overhead disable path
event_log_enabled = false (default) → push_event() is a no-op.
Enable via game.logger.enable_event_log() for puzzle runs.
MCTS and fuzz always run with events disabled.

### Rewind / determinism
- event_log NOT serialized (like log_buffer)
- truncate_events_to() for undo sync
- State hash does not include event_log

### Call sites wired
- SpellCast: priority.rs (spell cast path, !self.replaying)
- TriggerFired: actions/mod.rs (check_death_triggers, LeavesBattlefield triggers)
- CreatureDied: state.rs (lethal damage) + combat.rs (combat damage)
- LifeChanged: life.rs (positive delta from gain, negative from lose)

### Query API
game.logger.events() → EventLogView (zero-copy)
events.any_trigger_fired_from('Fecundity')
events.any_creature_died_named('Grizzly Bears')
events.any_spell_cast_named('Lightning Bolt')

## Tasks completed
- [x] LogEvent foundation + GameLogger integration
- [x] Benchmark proving no regression
- [x] Wire LifeChanged events in life.rs
- [x] Add AssertionKind variants: TriggerFired, SpellCast, CreatureDied, LifeGained
- [x] Update parser.rs for new assertion keywords
- [x] Update evaluator.rs: events parameter + new eval arms
- [x] Enable event_log in puzzle runner before run_game (puzzle_assert_e2e.rs run_puzzle)
- [x] Migrate Spirit Link tests #11/#12/#13 from log string matching to LifeChanged events
- [x] Add 'life gained ge N' assertions to spirit_link_*.pzl files
- [x] Unit tests for all new assertion kinds (parser + evaluator)
- [x] Update PUZZLE_ASSERTION_DSL.md

## Design doc
ai_docs/reference/PUZZLE_ASSERTION_DSL.md (updated to reflect Phase 2 status)

## Branch
claude/puzzle-trigger-assertions (slot02)
