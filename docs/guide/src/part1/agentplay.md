# Agent Play and Reproducers

You cannot type at the interactive terminal UI from a script, but DeepScry ships
a small Python toolchain under `agentplay/` that drives deterministic games one
choice at a time — either by hand, or by letting an AI agent make each choice.
Every session is replayable from a single self-contained shell script.

This chapter summarises the workflow; the authoritative, fuller version lives in
`docs/HOWTO_AGENTPLAY+REPRODUCERS.md`.

## The three entry points

| Script | Purpose |
| --- | --- |
| `agentplay/agent_game.py` | **Recommended.** End-to-end AI-driven game; an LLM (or a `--mock` random selector) makes each choice, with optional scenario / bug-detection prompting. |
| `agentplay/start_game.py` | Manually start a session: run up to the first choice, write the session files, print the menu. |
| `agentplay/continue_game.py` | Manually append one player's next choice and replay the whole game so far. |

All three produce the same on-disk session layout and the same reproducer
script, so you can switch freely between agent-driven and manual modes against
one session.

## Quick start

```bash
# AI vs AI, with built-in bug detection (the agent can STOP and emit a bug report)
./agentplay/agent_game.py -- decks/old_school/01_rogue_rogerbrand.dck decks/old_school/02_thedeck_peterschnidrig.dck

# Mock mode: local random choices, no API tokens spent — good for smoke tests
./agentplay/agent_game.py --mock --seed 42 -- decks/a.dck decks/b.dck

# Drive a puzzle instead of a normal game
./agentplay/agent_game.py --puzzle puzzles/bolt_test.pzl
```

> **Note:** the `--puzzle` flag here is a *script-level* convenience of
> `agent_game.py`; under the hood it launches the engine with
> `tui --start-state <file>`. The `mtg` binary itself has no `puzzle`
> subcommand.

Useful `agent_game.py` flags include `--scenario "<text>"` (keep a reproduction
target in the prompt every turn), `--mode {agent-vs-heuristic, agent-vs-random,
agent-vs-agent, random-vs-random}`, `--max-turns N`, `--p1-draw` / `--p2-draw`,
and `--seed N`. Run `./agentplay/agent_game.py --help` for the full list.

## Manual sessions

When you want a tight scripted reproducer (and don't want to spend agent
tokens), drive the game by hand:

```bash
# Start a session — runs up to the first choice and prints the menu
./agentplay/start_game.py decks/grizzly_bears.dck decks/royal_assassin.dck \
    --p1-draw="Forest;Grizzly Bears;Forest"

# Add choices one at a time; each call replays the whole game and stops at the
# next decision. The first argument selects which player's choice file to extend.
./agentplay/continue_game.py p1 "play mountain"
./agentplay/continue_game.py p1 "cast lightning bolt"
./agentplay/continue_game.py p1 "target bob"
./agentplay/continue_game.py p2 "pass"
```

Choices may be **numeric** (menu indices, e.g. `"0"`, `"3"` — simple but fragile
to menu reordering) or **rich text** (e.g. `"play mountain"`, `"cast lightning
bolt"` — robust to option ordering). You can mix both. The full input grammar is
covered in [Scripted Play](./scripted_play.md) and the
[Fixed-Input reference](../part3/fixed_input_syntax.md).

## Session directory layout

Sessions live under `agentplay/`, normally in a numbered `NNN.game/` directory.
Each session directory contains, among other files:

| File | Contents |
| --- | --- |
| `p1_choices.txt` / `p2_choices.txt` | Each player's choices, one per line, in order. |
| `initial_args.txt` | The original `mtg tui` argv. |
| `snapshot.json` / `game.snapshot` | Latest replayed state (JSON / binary). |
| `game.log` | Engine log from the last replay. |
| `reproduce_game.sh` | An executable script that replays the whole session deterministically. |

## Reproducers

`reproduce_game.sh` is regenerated after every choice and inlines a single
deterministic `mtg tui` command, for example:

```bash
cargo run --release --bin mtg -- tui decks/old_school/01_rogue_rogerbrand.dck decks/old_school/02_thedeck_peterschnidrig.dck \
    --p1=fixed --p2=fixed \
    --p1-fixed-inputs="0;1;pass;play swamp" \
    --p2-fixed-inputs="0;1;pass;play swamp" \
    --stop-on-choice=5 \
    --seed=42 --json --log-tail=100
```

This is exactly what you should paste into a bug report: it is self-contained,
deterministic, and survives session cleanup. Because reproducers replay **from
scratch** (same seed + same choices ⇒ same outcome) rather than loading a
snapshot, they remain valid across engine changes that don't alter behaviour.

Tips for good reproducers: start with the smallest decks that reproduce the
issue, pin the opening hand with `--p1-draw` / `--p2-draw`, trim trailing
choices so the script stops as soon as the bug fires, and re-run it once to
confirm determinism before filing.
