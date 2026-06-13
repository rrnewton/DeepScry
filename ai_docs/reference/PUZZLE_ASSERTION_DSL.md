# Puzzle Assertion DSL

**Tracking issue:** mtg-0oopj
**Status:** Phase 1 implemented (final-state assertions only)

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
               | zone_count_pred
               | zone_contains_pred
               | library_top_pred
               | game_result_pred
               | turn_pred

life_pred         ::= 'life' SPACE comparison
zone_count_pred   ::= zone SPACE 'count' SPACE comparison
zone_contains_pred::= zone SPACE 'contains' SPACE card_name
library_top_pred  ::= 'library' SPACE 'top' SPACE integer SPACE 'contains' SPACE card_name
game_result_pred  ::= 'game' SPACE ('won' | 'lost' | 'drawn' | 'ended')
turn_pred         ::= 'turn' SPACE comparison

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
```

---

## Typed AST

```rust
// In mtg-engine/src/puzzle/assert/mod.rs (feature = "puzzle-assert")

pub enum PlayerScope { Me, Opponent }
pub enum Comparator { Eq, Ne, Lt, Le, Gt, Ge }
pub enum Zone { Hand, Graveyard, Battlefield, Exile, Library }
pub enum GameResultPredicate { Won, Lost, Drawn, Ended }

pub enum AssertionKind {
    Life { scope: PlayerScope, cmp: Comparator, value: i32 },
    ZoneCount { scope: PlayerScope, zone: Zone, cmp: Comparator, value: usize },
    ZoneContains { scope: PlayerScope, zone: Zone, card_name: String },
    LibraryTopContains { scope: PlayerScope, depth: usize, card_name: String },
    GameResult(GameResultPredicate),
    TurnNumber { cmp: Comparator, value: u32 },
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

---

## Log-derived (event) assertions — DEFERRED to a later phase

The engine's `GameLogger` stores `LogEntry { message: String, category: Option<String>, ... }`.
There is no structured event enum: all game actions are formatted into human-readable strings
and placed in `message`. Asserting "trigger fired" / "creature died" / "spell cast" via
substring matching on `message` would violate the project's **NO HACKY STRING OPERATIONS ON
STRUCTURED DATA** rule.

**The correct fix**: add a `GameEvent` enum emitted alongside (not replacing) the existing
string log. Each event carries structured fields (e.g., `GameEvent::CardDied { card_id,
source_id }`). Assertions can then filter `&[GameEvent]` by variant without touching strings.

This is a non-trivial engine change (it touches `GameLogger` and every `gamelog()` call site).
It is NOT part of Phase 1. The gap is intentional and documented here so future implementors
know exactly what to add. Until a `GameEvent` stream exists, any log-derived assertions
in `.pzl` files will be silently skipped with a warning.

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
is absent). The puzzle runner calls `evaluate_assertions(&puzzle.assertions, &game, &result)`
after `run_game` returns and emits pass/fail lines for each assertion.

---

## Later phases (out of scope for Phase 1)

- **Phase 2: Structured GameEvent stream** — add `GameEvent` enum alongside the string log;
  implement log-derived assertions (trigger fired, creature died, spell cast, zone change).
- **Phase 3: Golden game-log oracle** — `[golden_log]` section with a hash of the expected
  log; one-command `--rebless` re-records it.
- **Phase 4: Bulk parallel runner** — `puzzle run --all` with cgroup-capped parallelism.
- **Phase 5: Rewind-determinism mode** — run each puzzle twice and diff the game log.
- **Phase 6: Migration** — move the 668 existing external Rust assertions into the DSL.
