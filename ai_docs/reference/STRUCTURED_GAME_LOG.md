# Structured Game Log

**Status:** Foundation implemented (SpellCast, TriggerFired, CreatureDied).
**Tracking issue:** (see mtg beads — search "structured log")

---

## Purpose

The puzzle assertion DSL (Phase 1, `claude/puzzle-format-phase1`) supports
final-state assertions only: life totals, zone contents, turn count. Phase 2
needs **event-level** assertions: "trigger X fired", "creature Y died",
"spell Z was cast". Querying the string log buffer for these would violate
the No-Hacky-String-Operations rule and is fragile to message-format changes.

The structured log is a parallel `Vec<LogEvent>` that records game events as
strongly-typed enum variants — the same pattern as `UndoLog` / `GameAction`.

---

## Architecture

### Files

| File | Role |
|------|------|
| `mtg-engine/src/game/log_event.rs` | `LogEvent` enum, `ZoneTag`, `DamageTarget`, `EventLogView` |
| `mtg-engine/src/game/logger.rs` | `GameLogger` — now has `event_log: Vec<LogEvent>` + `event_log_enabled: bool` |

### `LogEvent` enum (flat, no `Box`)

```rust
pub enum LogEvent {
    SpellCast   { card_id, card_name, caster }
    TriggerFired { source_id, source_name, controller, description }
    CreatureDied { card_id, card_name, controller }
    ZoneChange  { card_id, card_name, owner, from: ZoneTag, to: ZoneTag }
    DamageDealt { source_id, source_name, amount, target: DamageTarget }
    LifeChanged { player, delta, new_total }
    TurnStarted { turn_number, active_player }
    StepStarted { step, active_player }
}
```

Follows the `UndoLog/GameAction` pattern exactly: flat variants, no `Box`,
no per-entry heap allocation beyond owned `String` fields.

### Zero-overhead disable path

`GameLogger` has a boolean `event_log_enabled` (default `false`). The
`push_event` method is:

```rust
#[inline]
pub fn push_event(&mut self, event: LogEvent) {
    if self.event_log_enabled {
        self.event_log.push(event);
    }
}
```

When disabled: one branch instruction, no allocation, no string construction
at the call site (callers only call `push_event` AFTER the string has been
formatted for gamelog — there is no extra string work). Event logging is OFF by
default for all MCTS, fuzz, and benchmark runs.

Enable via:
```rust
game.logger.enable_event_log();   // before the game loop starts
```

---

## Benchmark Results

Measured with `cargo run --release --example bench_logging_overhead
--features network` (300 games × 3 configs, `decks/fuzz_bolt_mirror.dck`,
`HeuristicController` vs `HeuristicController`):

| Config | Per-game | vs Silent |
|--------|----------|-----------|
| Silent (logging OFF) | ~2.9–3.2 ms | baseline |
| Memory (string log ON, events OFF) | ~3.0–3.8 ms | +0–19% (median ~10%) |
| MemEvt (string log ON, events ON) | ~3.2–3.7 ms | noise-level vs Memory |

**Key findings:**
- String logging costs roughly **10%** of game-sim time in Normal+Memory mode
  (the mode WASM uses). High run-to-run variance (~2–19%) is system noise.
- **Event logging adds zero measurable overhead** on top of string logging,
  because SpellCast, TriggerFired, and CreatureDied events occur rarely (a few
  per game) and each push is just a `Vec::push` of a small enum.
- Silent mode (`VerbosityLevel::Silent`) removes the string-log overhead. It
  is the correct mode for MCTS rollouts and fuzz runs.

The simple guard in each logger method (`if VerbosityLevel::X > self.verbosity
&& !should_capture { return; }`) does short-circuit the `log_buffer.push` but
does NOT prevent the `format!()` string construction at call sites. A future
optimization would add `is_logging_active()` guards at call sites to skip
string formatting entirely when silent. This would require adding guards across
~100+ call sites in `logging.rs`, `priority.rs`, `state.rs` etc.

---

## Rewind / Determinism Decision

**The `event_log` is NOT serialized.** The `GameLogger` custom `Serialize`
impl (see `logger.rs`) skips both `log_buffer` and `event_log`. On
deserialization both start empty.

**Rewind truncation:** The string `log_buffer` is truncated by `truncate_to()`
when the undo log pops an action. The `event_log` has a parallel
`truncate_events_to(size)` method. Currently **callers do not track event log
sizes in the undo log** — this is intentional for Phase 1 of this feature:

- Puzzle assertions run at game END (after the puzzle script completes).
  At that point the game has not been rewound, so the event log is complete.
- Rewind operations during MCTS/fuzz have `event_log_enabled = false`, so
  no events accumulate at all.
- If we later need event assertions *during* a rewind-heavy path (e.g.
  assertions on a specific rewind + replay cycle), we must: (a) track
  `prior_event_log_size` in `UndoLog` alongside `prior_log_size`, and (b)
  call `truncate_events_to` on undo alongside `truncate_to`. This is the same
  pattern already used for the string buffer.

**State hash:** The event log does not participate in the game state hash
(`compute_state_hash`). It is an auxiliary output buffer, not game state.

---

## Call Sites (Phase 1 implementation)

Events recorded as of the initial implementation:

| Event | Where emitted | Condition |
|-------|--------------|-----------|
| `SpellCast` | `game_loop/priority.rs` (spell-cast path) | `!self.replaying` — not emitted during rewind replay |
| `TriggerFired` | `game/actions/mod.rs` (`check_death_triggers`) | When a `LeavesBattlefield` trigger fires |
| `CreatureDied` | `game/state.rs` (`check_lethal_damage`) | Creature dies from lethal damage (non-combat) |
| `CreatureDied` | `game/actions/combat.rs` (`apply_combat_damage_plan`) | Creature dies from combat damage |

Events defined but NOT yet emitted (added to `LogEvent` for the full schema,
wiring left for Phase 2):

- `ZoneChange` — most zone moves
- `DamageDealt` — at effect execution in `logging.rs`
- `LifeChanged` — at life-gain/loss sites
- `TurnStarted`, `StepStarted` — at turn/step transitions in `steps.rs`

---

## Query API

```rust
// Get an EventLogView (zero-copy borrow of the event slice):
let events = game.logger.events();

// Convenience query methods:
events.any_trigger_fired_from("Fecundity")
events.any_creature_died_named("Grizzly Bears")
events.any_spell_cast_named("Lightning Bolt")

// Or iterate manually:
for event in events.iter() {
    if let LogEvent::CreatureDied { card_name, .. } = event {
        println!("died: {}", card_name);
    }
}
```

---

## Future: Puzzle Assertion Integration (Phase 2)

To add event-level assertions to the `.pzl` DSL:

1. Add variants to `AssertionKind` (in `mtg-engine/src/puzzle/assert/mod.rs`):
   ```
   TriggerFired { source_name: String }
   SpellCast    { card_name: String }
   CreatureDied { card_name: String }
   ```
2. Add grammar in `parser.rs`:
   ```
   trigger fired <source-name>
   spell cast <card-name>
   creature died <card-name>
   ```
3. Enable the event log in the puzzle runner (`loader.rs` / the bulk runner)
   before calling `run_game`.
4. In `evaluator.rs`, query `game.logger.events()` for event-kind assertions.

The event log must be enabled BEFORE the puzzle game loop starts. It is
lightweight enough that puzzle runs can always enable it.

---

## MCTS / Fuzz Disable Flow

For MCTS and fuzz runs where replay is used:

1. Run the game with `VerbosityLevel::Silent` and `event_log_enabled = false`.
2. When a bug is detected, capture the seed.
3. Re-run the same game with `VerbosityLevel::Normal`,
   `OutputMode::Memory`, and `game.logger.enable_event_log()`.
4. Inspect the full string log and event log.

The event log can also be replayed by any deterministic replay (same seed +
same controller sequence → same events, since the engine is deterministic).
