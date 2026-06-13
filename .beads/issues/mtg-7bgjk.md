---
title: 'Puzzle DSL Phase 2: structured game-log events (LogEvent enum) for trigger/creature-death/spell-cast assertions'
status: open
priority: 3
issue_type: task
created_at: 2026-06-13T23:00:36.509627717+00:00
updated_at: 2026-06-13T23:00:36.509627717+00:00
---

# Description

## Purpose
Phase 2 of the puzzle assertion DSL. Unblocks event-level assertions (trigger fired, creature died, spell cast) by providing a strongly-typed structured log alongside the existing string log.

## Background
Phase 1 (mtg-0oopj, landed) implements final-state assertions only. Event assertions were deferred because the string log violates the NO HACKY STRING OPERATIONS rule. This issue tracks the structured log foundation that enables them.

## Implementation (landed on claude/puzzle-structured-log)

### Core files
- mtg-engine/src/game/log_event.rs — LogEvent enum (SpellCast, TriggerFired, CreatureDied, ZoneChange, DamageDealt, LifeChanged, TurnStarted, StepStarted)
- mtg-engine/src/game/logger.rs — GameLogger gains event_log: Vec<LogEvent> + enable_event_log_enabled: bool

### Zero-overhead disable path
event_log_enabled = false (default) → push_event() is a no-op.
Enable via game.logger.enable_event_log() for puzzle runs.
MCTS and fuzz always run with events disabled.

### Benchmark results (bench_logging_overhead example, 300 games × 3 configs)
- String logging: ~10% of game-sim time when ON in Normal+Memory mode
- Event logging adds: ~zero additional overhead (events are rare per game, push is cheap)
- Silent mode: removes ~10% overhead vs string-ON

### Rewind / determinism
- event_log NOT serialized (like log_buffer)
- Provides truncate_events_to() for future rewind sync
- Phase 1 puzzle assertions run at game END so no truncation needed now
- State hash does not include event_log

### Call sites wired (Phase 1 of structured log)
- SpellCast: priority.rs (spell cast path, !self.replaying)
- TriggerFired: actions/mod.rs (check_death_triggers, LeavesBattlefield triggers)
- CreatureDied: state.rs (lethal damage) + combat.rs (combat damage)

### Query API
game.logger.events() → EventLogView (zero-copy)
events.any_trigger_fired_from('Fecundity')
events.any_creature_died_named('Grizzly Bears')
events.any_spell_cast_named('Lightning Bolt')

## Next steps (Phase 2 wiring)
- [x] LogEvent foundation + GameLogger integration
- [x] Benchmark proving no regression
- [ ] Wire remaining events: ZoneChange, DamageDealt, LifeChanged, TurnStarted, StepStarted
- [ ] Add AssertionKind variants: TriggerFired, SpellCast, CreatureDied
- [ ] Update parser.rs + evaluator.rs for event assertions
- [ ] Enable event_log in puzzle runner before run_game
- [ ] Demo .pzl asserting 'trigger fired' from Fecundity or similar
- [ ] Wire into make validate / bulk runner (slot02: claude/puzzle-bulk-runner)

## Design doc
ai_docs/reference/STRUCTURED_GAME_LOG.md

## Branch
claude/puzzle-structured-log (validate green, pushed)
