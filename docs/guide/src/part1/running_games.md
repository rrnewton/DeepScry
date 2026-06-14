# Running Games

The `mtg tui` subcommand runs a single game. Despite the name ("text UI"), it
covers every single-game mode: a human playing in the terminal, two AIs playing
each other, a scripted reproducer, or a puzzle.

## A first game

The simplest invocation takes one or two deck files:

```bash
# Human (player 1) vs heuristic AI (player 2) — the defaults
mtg tui decks/old_school/01_rogue_rogerbrand.dck decks/old_school/02_thedeck_peterschnidrig.dck

# One deck file is used for both players if you give only one
mtg tui decks/simple_bolt.dck
```

By default player 1 is the interactive `tui` controller and player 2 is the
`heuristic` AI. You change who drives each seat with `--p1` and `--p2`.

## Controller types

Each seat is driven by a *controller*. The controller types (the
`ControllerType` enum in `mtg-engine/src/main.rs`) are:

| Controller | Behaviour |
| --- | --- |
| `zero` | Always picks the first meaningful action. Deterministic; good for smoke tests. |
| `random` | Makes random (but seeded, reproducible) choices. |
| `tui` | Interactive text UI for a human, reading from stdin. |
| `fancy` | Full-screen multi-panel terminal UI (ratatui). |
| `heuristic` | The strategic AI. |
| `fixed` | Replays a predetermined script of choices (see [Scripted Play](./scripted_play.md)). |
| `fancy-fixed` | Like `fixed`, but renders the fancy UI and can capture screenshots. |

Example — watch two AIs play:

```bash
mtg tui decks/a.dck decks/b.dck --p1 heuristic --p2 heuristic --seed 42
```

## Determinism and seeds

A game is fully determined by its deck files, its starting hands, the seed, and
the sequence of controller choices. The same inputs always produce the same
game. The relevant flags:

- `--seed N` — the master seed for the engine and the controllers. Pass
  `--seed from_entropy` for a non-deterministic game.
- `--seed-p1 N` / `--seed-p2 N` — override the per-controller seed.
- `--deck-seed N` — use a *separate* seed for the initial shuffle only, so you
  can sample different draws while keeping the same starting hands, or vice
  versa.
- `--p1-draw "Mountain;Lightning Bolt;Mountain"` / `--p2-draw "..."` — pin the
  opening hand (1–7 cards, semicolon-separated).

## Controlling output

- `--verbosity / -v <0..3>` — `0` silent, `1` minimal, `2` normal (default),
  `3` verbose.
- `--log-tail K` — only print the last `K` lines of the log at exit. Handy with
  `--stop-on-choice` to keep output a constant size.
- `--no-color-logs` — disable ANSI colour (also respects the `NO_COLOR` env
  var).
- `--json` — write snapshots in JSON instead of the binary format.
- `--tag-gamelogs` — prefix each official game-action log line with
  `[GAMELOG TurnN STEP]`, which makes it possible to diff a local game's log
  against a networked game's log line-for-line.

## Stopping, snapshotting, and resuming

You can stop a game part-way through and save its exact state:

- `--stop-on-choice N` — stop after `N` choices. `N:p1` / `N:p2` counts only
  one seat.
- `--stop-when-fixed-exhausted` — stop when a fixed-input script runs out
  (used to build reproducers incrementally).
- `--snapshot-output FILE` — where to write the snapshot (default
  `game.snapshot`).

Resume later with the `resume` subcommand:

```bash
# Capture a snapshot after 10 choices played by two heuristic AIs
mtg tui DECK1.dck DECK2.dck --seed 100 --stop-on-choice 10 \
    --p1 heuristic --p2 heuristic --snapshot-output game.snapshot

# Resume — by default it restores controllers, RNG state, and choices exactly.
mtg resume game.snapshot

# Resume but swap in fixed-input controllers for a hand-crafted finish
mtg resume game.snapshot \
    --override-p1 fixed --p1-fixed-inputs "0;1;2" \
    --override-p2 fixed --p2-fixed-inputs "0;1;2"
```

Snapshots preserve everything — game state, RNG state, controller state, turn
and choice counters — so a resume produces a byte-identical continuation. This
is the same machinery the engine uses internally for rewind and replay; see
[Snapshot and Replay](../part2/snapshot_architecture.md) in Part II.

## Tournaments

`mtg tourney` runs many games in parallel and reports aggregate statistics:

```bash
# 1000 games among three decks, mirror matches excluded
mtg tourney decks/a.dck decks/b.dck decks/c.dck --games 1000 --seed 42

# Run for a fixed wall-clock budget instead of a game count
mtg tourney decks/a.dck decks/b.dck --seconds 30

# Only mirror matches (each deck against itself)
mtg tourney decks/a.dck --mirror-only --games 200
```

`--games` and `--seconds` are mutually exclusive. Both seats default to the
`heuristic` AI.
