# The Puzzle (`.pzl`) Language

A puzzle file (`.pzl`) describes a pre-built Magic board state and, optionally, a
set of assertions about how a game played from that state should turn out.
Puzzles let the engine be tested against concrete scenarios — "from this board,
the AI should win by turn 2" — without writing per-scenario Rust.

This chapter is written **against the code**, not just the older design notes.
Where the existing reference documents (`ai_docs/reference/PZL_GRAMMAR.md` and
`ai_docs/reference/PUZZLE_ASSERTION_DSL.md`) disagree with what the parser and
runners actually do, this chapter follows the code and flags the difference in a
box like this:

> **Discrepancy (flagged):** an example of a place where a source doc and the
> code disagree.

The relevant code lives in `mtg-engine/src/puzzle/` (format, state, metadata,
and the `assert/` submodule) and in the test runners
`mtg-engine/tests/puzzle_bulk_runner.rs` and
`mtg-engine/tests/puzzle_golden_check.rs`.

## How a puzzle is loaded and run

There is **no `mtg puzzle` subcommand**. A puzzle is loaded by passing it as a
start state to `tui`:

```bash
mtg tui --start-state puzzles/bolt_test.pzl
```

For automated testing, two test runners discover and run puzzles in bulk; see
[Running and blessing puzzles](#running-and-blessing-puzzles) below. Both runners
drive **both seats with the heuristic AI** at a fixed seed — there is no
in-puzzle scripting of moves.

> **Discrepancy (flagged):** older notes describe puzzle "controller commands" or
> a scripted-action section. **There is none.** The `.pzl` format sets up a board
> and (optionally) assertions; it does not script the moves. The only "who plays"
> knob is the `HumanControl` metadata flag, which is stored but not consulted by
> the bulk runners. To script moves, use the fixed-input controller
> (see [Scripted Play](../part1/scripted_play.md)) on a normal game.

## File format

A `.pzl` file is an INI-style file: lines are grouped under `[section]` headers,
blank lines and `#`-comment lines are ignored, and within a section each line is
a `key: value` or `key = value` pair (both separators are accepted). Parsing is
done by tokenised splitting, never substring matching. The parser is in
`mtg-engine/src/puzzle/format.rs`.

The recognised sections are `[metadata]`, `[state]`, and (when the
`puzzle-assert` feature is on) `[assertions]`. Unknown sections are kept but
ignored, for forward compatibility.

> **Discrepancy (flagged):** `PZL_GRAMMAR.md` says **both** `[metadata]` and
> `[state]` are required. In the code, **only `[state]` is required** — its
> absence is the one hard error ("Missing [state] section in puzzle file",
> `format.rs`). `[metadata]` is optional and defaults are used when it is absent.

> **Discrepancy (flagged):** `PZL_GRAMMAR.md` calls the parser a "manual
> recursive descent parser" over a formal grammar and quotes line counts and a
> "100% success on 351 files" benchmark. The actual parser is a flat,
> line-oriented INI splitter, and the corpus is now larger (hundreds of
> forge-java files, with some documented load failures). Treat the grammar
> document's parser-internals and benchmark sections as out of date.

### `[metadata]` section

Optional key-value descriptive data. Keys are case-insensitive (lowercased
internally): `name`, `url`, `goal`, `turns`, `difficulty`, `description`,
`targets`, `targetcount`, `humancontrol`. The parser in
`mtg-engine/src/puzzle/metadata.rs` defines the accepted values.

> **Discrepancy (flagged):** the exact goal strings the code accepts differ
> slightly from `PZL_GRAMMAR.md` — for example the code accepts both
> `destroy specified permanents` and an undocumented alias
> `destroy specified creatures`, and the win-race goal is spelled
> `win before opponent's next turn` (the doc drops the `'s next`). When authoring
> goals, check `metadata.rs` rather than the grammar doc.

### `[state]` section

The required section. It sets the turn, the active player, the active phase, and
per-player zones. The parser is `mtg-engine/src/puzzle/state.rs`.

```ini
[state]
turn = 3
activeplayer = p0
activephase = MAIN1

p0life = 20
p0hand = Lightning Bolt; Mountain
p0battlefield = Mountain|Tapped; Grizzly Bears
p1life = 5
p1battlefield = Llanowar Elves
```

Per-player lines use a `p0` / `p1` prefix followed by the field name. The
recognised fields are the scalar/counter fields `life`, `landsplayed`,
`landsplayedlastturn`, `counters`, `manapool`, `persistentmana`, and the zone
names `hand`, `battlefield`, `graveyard`, `library`, `exile`, `command` (see the
`match field` arm in `state.rs`). Zone contents are semicolon-separated card
notations. (`command` is parsed but currently dropped at load time — see the
note below.)

> **Discrepancy (flagged):** the grammar doc lists `human` / `ai` as valid
> per-player prefixes. In the code, per-player state lines only recognise the
> **`p0` / `p1`** prefixes (the parser strips a 2-char prefix). `human` / `ai`
> work only for the `activeplayer` line, not for `…life` / `…hand` lines — so a
> puzzle that writes `humanlife=` will not load that line as intended.
>
> The `command` zone is a subtler case: the `[state]` parser **does** recognise
> `p0command=` and parses it into a `command: Vec<CardDefinition>` field (see the
> `command` arms in `state.rs`). However, the **loader currently drops it**,
> because the runtime game state (`PlayerZones`) has no command zone yet (the
> puzzle `README.md` notes "Command zone not yet in PlayerZones (requires
> architecture change)"). So `p0command=` parses cleanly but has no effect on the
> loaded board until a command zone is added to the runtime.

> **Discrepancy (flagged):** the bulk runner's own comments cite some load
> failures as `Unknown phase: DECLAREATK` and `Unknown counter type: TIME`.
> Checked against code: the phase abbreviation `DECLAREATK` is indeed **not**
> accepted (use `DECLAREATTACKERS`), but `TIME` **is** a valid counter type
> (`core/types.rs` maps it to `CounterType::Time`) — so that particular
> root-cause note is stale.

### Card notation

Within a zone list, each card may carry pipe-separated modifiers, e.g.
`Mountain|Tapped` or `Grizzly Bears|Counters:P1P1=2`. Boolean modifiers (such as
`Tapped`, `SummonSick`, `FaceDown`) and key-value modifiers (such as `Id`,
`Counters`, `Damage`, `AttachedTo`) are parsed by
`mtg-engine/src/puzzle/card_notation.rs`. Unknown modifiers are ignored for
forward compatibility.

## The assertion DSL

When the engine is built with the `puzzle-assert` Cargo feature, a `[assertions]`
section makes a puzzle **self-checking**: the expected outcome is written inside
the `.pzl` file, so any runner can verify it without separate per-puzzle Rust.
With the feature off, the section is parsed and stored but the evaluator is
compiled out entirely (zero runtime overhead).

Each non-blank, non-comment line in `[assertions]` is one assertion. The grammar
(parser at `mtg-engine/src/puzzle/assert/parser.rs`):

```text
assertion    ::= 'NOT'? scope? predicate
scope        ::= 'me' | 'opponent'        (default: me = the puzzle's p0)
predicate    ::= <keyword> ...            (keyword selects the assertion kind)
```

A predicate always starts with one of the leading keywords the parser
recognises (see the error enumeration at the bottom of `parse_predicate` in
`parser.rs`):

```text
life | hand | graveyard | battlefield | exile | library
   | game | turn | trigger | spell | creature
```

The `life` keyword leads two different predicates: `life <cmp> <int>` (the
final life total) and `life gained <cmp> <int>` (a life-gain event count — see
the event assertions below). The keywords `trigger`, `spell`, and `creature`
all lead event-backed predicates.

The assertion **kinds** are exactly the variants of `AssertionKind` in
`mtg-engine/src/puzzle/assert/mod.rs`, evaluated by
`assert/evaluator.rs`:

The first six kinds read **final game state** or the **game result**. The last
four are **event-backed**: they read the structured event log (see
[Event-backed assertions](#event-backed-assertions) below), and the table marks
their data source accordingly.

| Kind | Syntax | Checks | Data source |
| --- | --- | --- | --- |
| `Life` | `life <cmp> <int>` | a player's life total | final state |
| `ZoneCount` | `<zone> count <cmp> <int>` | number of cards in a zone | final state |
| `ZoneContains` | `<zone> contains <card name>` | a named card is present in a zone (case-insensitive) | final state |
| `LibraryTopContains` | `library top <N> contains <card name>` | a named card is among the top N of the library | final state |
| `GameResult` | `game won` / `lost` / `drawn` / `ended` | the game's result | game result |
| `TurnNumber` | `turn <cmp> <int>` | number of turns played | game result |
| `TriggerFired` | `trigger fired` / `trigger fired from <card name>` | a triggered ability fired (optionally from a named source) | event log |
| `SpellCast` | `spell cast` / `spell cast <card name>` | a spell was cast (optionally a named one) | event log |
| `CreatureDied` | `creature died` / `creature died <card name>` | a creature died (optionally a named one) | event log |
| `LifeGained` | `life gained <cmp> <int>` | total life a player gained over the game | event log |

Where `<zone>` is one of `hand`, `graveyard`, `battlefield`, `exile`, `library`
(the `AssertZone` enum), and `<cmp>` is one of `eq`, `ne`, `lt`, `le`, `gt`,
`ge`. For the event-backed kinds, the trailing `<card name>` is optional: an
empty name matches **any** trigger / spell / creature death, and matching is
**case-insensitive**.

```ini
[assertions]
# P0 (me) must end at 20 life
life eq 20
# Opponent took damage
opponent life lt 20
# At least three permanents on my battlefield
battlefield count ge 3
# Lightning Bolt ended in my graveyard
me graveyard contains Lightning Bolt
# A specific card is on top of my library
library top 1 contains Forest
# I won the game
game won
# ...within two turns
turn le 2
# And I did NOT lose
NOT game lost
# A Lightning Bolt spell was cast at some point (event-backed)
spell cast Lightning Bolt
# Some trigger fired (event-backed, any source)
trigger fired
# My opponent gained at least 5 life over the game (event-backed)
opponent life gained ge 5
```

### What backs each assertion

The first six kinds (`Life`, `ZoneCount`, `ZoneContains`, `LibraryTopContains`,
`GameResult`, `TurnNumber`) read **final game state** (life totals, zone
contents, library order) or the **game result** (winner / turns played). The
evaluator reads these from `GameState` and `GameResult` directly.

### Event-backed assertions

The remaining four kinds are **event-backed**: instead of inspecting the final
board, they ask "did this *happen* at any point during the game?" by scanning the
structured event log. They are **live and wired into the DSL** (this was the
puzzle "Phase 2" feature):

| Syntax | Matches |
| --- | --- |
| `trigger fired` | any triggered ability fired |
| `trigger fired from <card name>` | a trigger fired from the named source |
| `spell cast` | any spell was cast |
| `spell cast <card name>` | the named spell was cast |
| `creature died` | any creature died |
| `creature died <card name>` | the named creature died |
| `life gained <cmp> <int>` | a player's total life gained (scope-aware) compares to `<int>` |

How they behave:

- **Names are case-insensitive**, and an **empty name means "any"**: `spell
  cast` matches any spell, `spell cast Lightning Bolt` matches only that spell.
- `life gained` is **scope-aware** like the other player-scoped predicates, so
  `opponent life gained ge 5` checks the opponent's total life gained. It sums
  the positive life-change deltas recorded in the event log.
- These kinds **consume the event log**: the evaluator scans an
  `EventLogView<'_>` over the engine's `LogEvent` stream (defined in
  `mtg-engine/src/game/log_event.rs`). The variants matched are
  `LogEvent::TriggerFired`, `SpellCast`, `CreatureDied`, and (for `life
  gained`) `LifeChanged`.
- The puzzle **runners always enable the event log**, so these assertions work
  out of the box: the e2e runner enables it explicitly, and the bulk runner
  passes `final_game.logger.events()` into the evaluator. If a run is performed
  *without* the event log, an event assertion fails with the error message
  **"event log not enabled for this puzzle run"** rather than silently passing.

> **Historical note:** an earlier draft of this guide (and the assertion-DSL
> design doc) described these event assertions as a *future* family that "does
> not exist yet" and named the event type `GameEvent`. That is now stale: the
> event assertions shipped, and the real event type is **`LogEvent`** (viewed
> through `EventLogView`). The design doc's `GameEvent` name was never the
> shipped one.

A couple of smaller behavioural notes the docs omit, confirmed in `evaluator.rs`:

- `library top N contains X` clamps `N` to the library size if `N` is larger
  than the library, rather than erroring.
- `game ended` is true only when there is a winner or an explicit draw; a game
  that stops by hitting the turn limit with no winner is **not** counted as
  `ended` (there is a dedicated test for this).

## Running and blessing puzzles

Two test runners exercise puzzles, wired to Make targets in the repository
`Makefile`:

- **`make puzzle-bulk-check`** runs `mtg-engine/tests/puzzle_bulk_runner.rs`. It
  discovers every `.pzl` under `test_puzzles/`, `puzzles/`,
  `forge-java/forge-gui/res/puzzle`, and `forge-java/forge-gui/res/tutorial`,
  runs each with two heuristic AIs at a fixed seed, evaluates any `[assertions]`
  (puzzles with none just smoke-test that they load and run), and writes a JUnit
  XML report. It runs in parallel, bounded to the CPU count, and gates against a
  known-bad baseline of panics / assertion failures / load errors.

- **`make puzzle-golden-check`** runs `mtg-engine/tests/puzzle_golden_check.rs`.
  For **locally-authored** puzzles only (`test_puzzles/` and `puzzles/`; the
  forge-java corpus is excluded because it has many pre-existing panics), it
  captures the game's text log and diffs it against a committed golden file at
  `test_puzzles/goldens/<stem>.golden.log` or `puzzles/goldens/<stem>.golden.log`.
  A mismatch fails the check.

- **`make puzzle-bless`** re-records every golden log from the current engine
  output (it runs the golden test with `MTG_BLESS_GOLDEN=1`). Use it after an
  *intentional* change to the log format, then review the diff
  (`git diff test_puzzles/goldens/ puzzles/goldens/`) before committing.

> **Discrepancy (flagged):** the assertion-DSL doc describes the golden mechanism
> as a `[golden_log]` section carrying a hash inside the `.pzl` file, refreshed
> with a `--rebless` flag. The **implemented** mechanism is different: separate
> `goldens/*.golden.log` files holding the full text log, refreshed via the
> `MTG_BLESS_GOLDEN=1` environment variable (the `make puzzle-bless` target).
> There is no `[golden_log]` section. The golden oracle compares the **text** log
> buffer, not the structured event stream.

> **Status note for the phased plan.** The assertion-DSL doc is organised as
> phases (final-state assertions → event stream → golden oracle → bulk runner →
> rewind diff → migrate external assertions) and still labels itself "Phase 1
> implemented." In reality much more has landed: the **final-state assertions**,
> the **event-backed assertions** (the four `trigger fired` / `spell cast` /
> `creature died` / `life gained` kinds documented above — the puzzle "Phase 2"
> work), the **golden oracle**, and the **bulk parallel runner** are all live. So
> the doc *understates* progress on those fronts while *overstating* the
> `[golden_log]`-hash design it never shipped. This guide's chapter reflects the
> code as it stands.
