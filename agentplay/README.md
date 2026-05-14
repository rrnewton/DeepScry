# Agentplay: Step-by-Step Game Testing

Scripts for playing MTG games step-by-step with scripted choices, useful for testing game engine behavior.

## Directory Layout

```
agentplay/
  agent_game.py      # Main entry: agent-driven game loop (Claude or --mock)
  start_game.py      # CLI: start a new game session
  continue_game.py   # CLI: append one choice and replay
  test_agent_game.py # Tests
  lib/               # Library modules (engine, prompts, card_defs)
```

## Quick Start

```bash
# Start a new game (creates numbered directory like 001.game)
./agentplay/start_game.py decks/simple_bolt.dck decks/simple_bolt.dck

# Continue with choices (specify player and choice)
./agentplay/continue_game.py p1 "play Mountain"
./agentplay/continue_game.py p2 "0"

# Use a specific game directory
./agentplay/start_game.py --game-dir=my_test.game decks/a.dck decks/b.dck
./agentplay/continue_game.py --game-dir=my_test.game p1 "1"
```

## Agent-Driven Play

```bash
# Run a full bug-finding agent game (Claude picks or stops with BUG_REPORT)
./agentplay/agent_game.py -- decks/simple_bolt.dck decks/simple_bolt.dck

# Keep a reproduction target in the prompt at every decision
./agentplay/agent_game.py \
  --scenario "Play until P2 attacks with a flying creature, then try to double-block and cast an instant combat trick" \
  -- decks/booster_draft/avatar/eric_avatar_draft.dck decks/booster_draft/avatar/gabriel_avatar_draft.dck

# Pure play mode disables STOP/BUG_REPORT prompting
./agentplay/agent_game.py --pure-play -- decks/simple_bolt.dck decks/simple_bolt.dck

# Mock mode (random choices, no API tokens burned)
./agentplay/agent_game.py --mock --seed 42 -- decks/simple_bolt.dck decks/simple_bolt.dck
```

### Engine driver: persistent vs stop-and-go

`agent_game.py` supports two engine driver modes via `--driver`:

| `--driver`     | Engine subprocess                                       | LLM session                              |
|----------------|---------------------------------------------------------|------------------------------------------|
| `persistent`   | ONE long-running `mtg tui --p1=tui --p2=<X>` process    | Per-player `claude --resume <session>`   |
| `stop-and-go`  | Re-runs `mtg tui --p1=fixed --p2=fixed` per decision    | Per-decision `claude -p <prompt>`        |

Both modes produce the same on-disk artefacts (see "Game Directory
Structure" below) so a game played in persistent mode can be replayed in
stop-and-go mode (and vice-versa). The default is `persistent`.

```bash
# Default — persistent driver, per-player resume sessions
./agentplay/agent_game.py -- decks/simple_bolt.dck decks/simple_bolt.dck

# Force the legacy stop-and-go driver (one mtg tui invocation per choice)
./agentplay/agent_game.py --driver=stop-and-go -- decks/simple_bolt.dck decks/simple_bolt.dck

# Persistent driver, but use one-shot `claude -p` per turn instead of `--resume`
./agentplay/agent_game.py --persistent-claude=oneshot -- decks/simple_bolt.dck decks/simple_bolt.dck
```

Persistent mode requires the engine to be built with the
`--tui-snapshot-path` flag (added in `mtg-engine/src/main.rs`) so the
Python harness can read the same structured `GameSnapshot` JSON between
choices that stop-and-go mode reads from `--snapshot-output`. If
`AGENTPLAY_FORCE_ONESHOT=1` is set in the environment, the
`ClaudeResumeSession` falls back to one-shot mode.

Bug-detection mode is enabled by default. Each agent prompt includes:
- the current game state,
- the full game log interleaved with prior choices and rationale,
- the game log since the last decision,
- the previous decision recap,
- the current menu of choices,
- any `--scenario` text.

In bug-detection mode the agent can either put a choice number on the final line
or write `STOP` with a `BUG_REPORT` section describing the suspected rules or
engine bug. Use `--no-bug-detection` or `--pure-play` for choice-only play.

## Documentation

For detailed documentation, see:
- **[docs/HOWTO_AGENTPLAY+REPRODUCERS.md](../docs/HOWTO_AGENTPLAY+REPRODUCERS.md)** - Full guide to agentplay system and reproducers

## Game Directory Structure

Each game directory (e.g., `001.game/`) contains:
- `p1_choices.txt` - Player 1's choices (one per line)
- `p2_choices.txt` - Player 2's choices (one per line)
- `snapshot.json` - Current game state (JSON format)
- `initial_args.txt` - Original command arguments
- `enriched_log.md` - Game log with agent reasoning (agent_game.py only)
- `bug_reports.log` - STOP/BUG_REPORT entries from bug-detection mode, when any
