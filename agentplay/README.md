# Agentplay: Step-by-Step Game Testing

This directory contains scripts for playing MTG games step-by-step with scripted choices, useful for testing game engine behavior without writing code.

## Quick Start

```bash
# Start a new game (creates numbered directory like 001.game and symlinks to current.game)
./agentplay/start_game.sh decks/booster_draft/spiderman/ryan_spiderman_draft.dck decks/booster_draft/spiderman/ryan_spiderman_draft.dck

# Continue with choices (auto-detects whose turn)
./agentplay/continue_game.sh "1"    # Choose action at index 1
./agentplay/continue_game.sh "0"    # Pass priority

# Or specify player explicitly
./agentplay/continue_game.sh --p1 "1"
./agentplay/continue_game.sh --p2 "0"
```

## Numbered Games

Games are automatically numbered (001.game, 002.game, etc.) and `current.game` is a symlink to the latest game. This allows you to:
- Keep a history of all test games
- Work on multiple games in parallel with `--game-dir`
- Easily reference the current game without remembering numbers

## Parallel Game Sessions

Use `--game-dir` to work on multiple games simultaneously without conflicts:

```bash
# Start a specific game
./agentplay/start_game.sh --game-dir=my_test.game decks/simple_bolt.dck decks/simple_bolt.dck

# Continue it (doesn't affect current.game)
./agentplay/continue_game.sh --game-dir=my_test.game "1"
```

## Documentation

For detailed documentation, see:
- **[docs/HOWTO_AGENTPLAY+REPRODUCERS.md](../docs/HOWTO_AGENTPLAY+REPRODUCERS.md)** - Full guide to agentplay system and reproducers

## Game Directory Structure

Each game directory (e.g., `001.game/`, `current.game -> 001.game`) contains:
- `p1_choices.txt` - Player 1's choices (one per line)
- `p2_choices.txt` - Player 2's choices (one per line)
- `game.snapshot` - Current game state (JSON format)
- `initial_args.txt` - Original command arguments
- `reproduce_game.sh` - Executable script to replay this exact game
