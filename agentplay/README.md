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
# Run a full agent game (Claude picks each action)
./agentplay/agent_game.py -- decks/simple_bolt.dck decks/simple_bolt.dck

# Mock mode (random choices, no API tokens burned)
./agentplay/agent_game.py --mock --seed 42 -- decks/simple_bolt.dck decks/simple_bolt.dck
```

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
