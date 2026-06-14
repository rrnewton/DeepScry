# Puzzle Assertion DSL

**Tracking issue:** mtg-935
**Status:** Phase 1 + Phase 2 implemented (final-state + event-log assertions)

This document is the evergreen specification for the inline assertion DSL added
to `.pzl` puzzle files. Keep it up to date as phases land.

---

## Purpose

The assertion DSL makes puzzles **self-checking**: expected outcomes are written
*inside* the `.pzl` file so any runner (CLI, integration test, bulk harness) can
verify them without separately-maintained Rust code per puzzle. This also reduces
test-maintenance drift — the assertion lives next to the state it checks.

---

## Feature gate

All assertion code lives under the `puzzle-assert` cargo feature. When the feature
is OFF the `[assertions]` section is parsed and stored but the evaluator is
compiled out entirely — the engine hot path never checks an "is an assertion
watching" flag. Zero runtime overhead when the feature is off.

```toml
cargo build                          # feature off (default) — zero overhead
cargo build --features puzzle-assert # feature on — parser + evaluator active
```

The `puzzle-assert` feature does NOT depend on `native` — it compiles without
async I/O or filesystem access, making it usable in unit tests and WASM contexts
that have puzzle strings in memory.

---

## Grammar (`[assertions]` section)

Each non-blank, non-comment line in `[assertions]` is one assertion. Lines
starting with `#` are comments. Parsing is token-based (no substring matching on
structured fields).

```ebnf
assertion    ::= negation? player_scope? predicate
negation     ::= 'NOT' SPACE
player_scope ::= ('me' | 'opponent') SPACE
predicate    ::= life_pred
               | life_gained_pred
               | zone_count_pred
               | zone_contains_pred
               | library_top_pred
               | game_result_pred
               | turn_pred
               | trigger_pred
               | spell_cast_pred
               | creature_died_pred

life_pred         ::= 'life' SPACE comparison
life_gained_pred  ::= 'life' SPACE 'gained' SPACE comparison
zone_count_pred   ::= zone SPACE 'count' SPACE comparison
zone_contains_pred::= zone SPACE 'contains' SPACE card_name
library_top_pred  ::= 'library' SPACE 'top' SPACE integer SPACE 'contains' SPACE card_name
game_result_pred  ::= 'game' SPACE ('won' | 'lost' | 'drawn' | 'ended')
turn_pred         ::= 'turn' SPACE comparison
trigger_pred      ::= 'trigger' SPACE 'fired' (SPACE 'from' SPACE card_name)?
spell_cast_pred   ::= 'spell' SPACE 'cast' (SPACE card_name)?
creature_died_pred::= 'creature' SPACE 'died' (SPACE card_name)?

zone        ::= 'hand' | 'graveyard' | 'battlefield' | 'exile' | 'library'
comparison  ::= comparator SPACE integer
comparator  ::= 'eq' | 'ne' | 'lt' | 'le' | 'gt' | 'ge'
card_name   ::= any non-empty text (case-insensitive, must match card's canonical name)
integer     ::= [0-9]+
```

Default player scope when omitted: `me` (the puzzle's P0 / "human" player).

### Example assertion lines

```ini
[assertions]
# P0 must end at 20 life
life eq 20

# P1 (opponent) must have taken damage
opponent life lt 20

# Three permanents on P0's battlefield
battlefield count ge 3

# Lightning Bolt ended up in the graveyard
me graveyard contains Lightning Bolt

# Opponent has nothing in hand
opponent hand count eq 0

# Specific card on top of library
library top 1 contains Forest

# P0 won the game
game won

# Game ended within 2 turns
turn le 2

# Negation: P0 did NOT lose
NOT game lost

# Spirit Link triggered at least once (any trigger)
trigger fired

# Spirit Link specifically triggered
trigger fired from Spirit Link

# Any spell was cast this game
spell cast

# A specific spell was cast
spell cast Lightning Bolt

# No Lightning Bolt was cast
NOT spell cast Lightning Bolt

# A creature died
creature died

# A specific creature died
creature died Grizzly Bears

# P0 gained at least 3 life (sum of LifeChanged positive deltas)
life gained ge 3

# Opponent gained no life
opponent life gained eq 0
```

> **Note:** Event-log assertions (`trigger fired`, `spell cast`, `creature died`,
> `life gained`) require the event log to be enabled before the game runs:
> `game.logger.enable_event_log()`. The `run_puzzle` helper in
> `puzzle_assert_e2e.rs` enables it automatically. If `events` is `None` (event
> log not enabled), the assertion fails with the message
> `"event log not enabled for this puzzle run"`.

---

## Typed AST

```rust
// In mtg-engine/src/puzzle/assert/mod.rs (feature = "puzzle-assert")

pub enum PlayerScope { Me, Opponent }
pub enum Comparator { Eq, Ne, Lt, Le, Gt, Ge }
pub enum Zone { Hand, Graveyard, Battlefield, Exile, Library }
pub enum GameResultPredicate { Won, Lost, Drawn, Ended }

pub enum AssertionKind {
    // Phase 1: final-state assertions (no event log required)
    Life { scope: PlayerScope, cmp: Comparator, value: i32 },
    ZoneCount { scope: PlayerScope, zone: Zone, cmp: Comparator, value: usize },
    ZoneContains { scope: PlayerScope, zone: Zone, card_name: String },
    LibraryTopContains { scope: PlayerScope, depth: usize, card_name: String },
    GameResult(GameResultPredicate),
    TurnNumber { cmp: Comparator, value: u32 },
    // Phase 2: event-log assertions (require game.logger.enable_event_log())
    TriggerFired { source_name: String },   // empty = any trigger
    SpellCast { card_name: String },        // empty = any spell
    CreatureDied { card_name: String },     // empty = any creature death
    LifeGained { scope: PlayerScope, cmp: Comparator, value: i32 }, // sum of positive LifeChanged deltas
}

pub struct Assertion {
    pub negated: bool,
    pub kind: AssertionKind,
    /// Original source line, for error reporting
    pub source_line: String,
}
```

`CardModifier` is the existing vocabulary for card notation in `.pzl` files. The
assertion DSL does not invent a second card-syntax: card names are plain text
(matching against `Card::name` via the existing `EntityStore`). Counter-level
assertions use `CounterType` from the existing core module.

---

## Data sources

| Assertion kind | Data source | Notes |
|---|---|---|
| `life` | `GameState::get_player(id)?.life` | Existing `Player::life` field |
| `ZoneCount` hand/graveyard/exile/library/command | `GameState::get_player_zones(id)?.<zone>.len()` | Existing `PlayerZones` fields |
| `ZoneCount` battlefield | Filter `GameState::battlefield.cards` by controller | Shared battlefield |
| `ZoneContains` | Same zones, filter by `Card::name` | Via `EntityStore::get` |
| `LibraryTopContains` | Library cards slice `[0..depth]` | Library order preserved |
| `GameResult` | `GameResult::winner`, `::end_reason` | Passed in after `run_game` |
| `TurnNumber` | `GameResult::turns_played` | Passed in after `run_game` |
| `TriggerFired` | `EventLogView::any_trigger_fired_from` | Queries `LogEvent::TriggerFired` |
| `SpellCast` | `EventLogView::any_spell_cast_named` | Queries `LogEvent::SpellCast` |
| `CreatureDied` | `EventLogView::any_creature_died_named` | Queries `LogEvent::CreatureDied` |
| `LifeGained` | Sum of `LogEvent::LifeChanged { delta > 0 }` for player | Wired in `life.rs` `execute_gain_life*` |

---

## Event-log assertions (Phase 2 — implemented)

Phase 2 adds `LogEvent` variants to `GameLogger` (in `game/log_event.rs`) that record
structured events alongside the human-readable string log. These events are queried by the
Phase 2 assertion kinds above.

**Wired event types:**

| `LogEvent` variant | Emitted by |
|---|---|
| `SpellCast { card_id, card_name, caster }` | `game_loop/priority.rs` |
| `TriggerFired { source_id, source_name, controller, description }` | `game/actions/mod.rs` |
| `CreatureDied { card_id, card_name, controller }` | `game/actions/combat.rs`, `game/state.rs` |
| `LifeChanged { player, delta, new_total }` | `game/actions/effects/life.rs` (positive delta from gain, negative from lose) |

**Zero-overhead disable:** The event log is off by default. `push_event` is a no-op when
`event_log_enabled = false`. Only puzzle runs and targeted tests call `enable_event_log()`.

**Rewind safety:** The event log is truncated alongside the string log via
`GameLogger::truncate_events_to` on undo, preserving rewind determinism.

**Call pattern:**

```rust
game.logger.enable_event_log();
let result = game_loop.run_game(&mut c0, &mut c1)?;
let events = game.logger.events(); // EventLogView<'_>
let report = evaluate_assertions(&puzzle.assertions, &game, &result, Some(&events));
```

---

## Integration / isolation

```
mtg-engine/src/puzzle/
├── mod.rs              — unchanged public API; re-exports AssertionSet when feature active
├── format.rs           — parse_puzzle() already returns extra sections; assertions extracted here
├── metadata.rs         — unchanged
├── state.rs            — unchanged
├── card_notation.rs    — unchanged (CardModifier reused by reference in assertions)
└── assert/
    ├── mod.rs          — AssertionSet, Assertion, AssertionError (all #[cfg(feature = "puzzle-assert")])
    ├── parser.rs       — parse_assertions(&[String]) -> Result<Vec<Assertion>>
    └── evaluator.rs    — evaluate(assertions, game, result) -> AssertionReport
```

The evaluator is a **library function** — it takes `&GameState` and `&GameResult` and returns
`AssertionReport { passed: Vec<&Assertion>, failed: Vec<AssertionFailure> }`. It has no side
effects; the runner (CLI or test) decides what to do with the report.

---

## Wire-up

`PuzzleFile` gains an `assertions` field (empty `Vec` when the feature is off or the section
is absent). The puzzle runner calls
`evaluate_assertions(&puzzle.assertions, &game, &result, events)` after `run_game` returns
and emits pass/fail lines for each assertion. Pass `Some(&game.logger.events())` to enable
event-log assertions, or `None` to skip them (Phase 1 assertions still run).

---

## Roadmap

- **Phase 1** (done): final-state assertions (`life`, `zone count/contains`, `game`, `turn`).
- **Phase 2** (done): event-log assertions (`trigger fired`, `spell cast`, `creature died`,
  `life gained`). `LifeChanged` events wired. Spirit Link tests #11/#12/#13 migrated.
- **Phase 3: Golden game-log oracle** — `[golden_log]` section with a hash of the expected
  log; one-command `--rebless` re-records it.
- **Phase 4: Bulk parallel runner** — `puzzle run --all` with cgroup-capped parallelism.
- **Phase 5: Rewind-determinism mode** — run each puzzle twice and diff the game log.
- **Phase 6: Migration** — move the 668 existing external Rust assertions into the DSL.
