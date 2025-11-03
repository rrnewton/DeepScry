# HOWTO: Play MTG Games and Build Reproducers (For AI Agents)

It's important that you have the experience of playing the MTG game we're developing so you can see actual issues with gameplay and compare to expected behavior given MTG rules. Testing in code is insufficient.

## Quick Start: Using the agentplay Scripts (Recommended)

The easiest way to play games step-by-step and build reproducers is using the `agentplay/` wrapper scripts:

### Starting a New Game Session

```bash
# Start with any valid mtg tui arguments
./agentplay/start_game.sh decks/simple_bolt.dck decks/simple_bolt.dck

# Or with specific initial hands:
./agentplay/start_game.sh decks/grizzly_bears.dck decks/royal_assassin.dck \
    --p1-draw="Forest;Grizzly Bears;Forest"

# Or from a puzzle state:
./agentplay/start_game.sh --start-state="puzzles/bolt_test.pzl"
```

This will:
1. Initialize the game with deterministic seed (42)
2. Stop before the first choice is needed
3. Show you the available actions
4. Save a snapshot to `agentplay/game.snapshot`
5. Print a REPRODUCER command for easy replay

### Adding Choices One at a Time

After start_game.sh shows you the available choices, add them one at a time:

```bash
# Add a choice (game determines whose turn it is)
./agentplay/continue_game.sh "0"

# The game will show the next available choices
# Continue adding choices as needed:
./agentplay/continue_game.sh "1"
./agentplay/continue_game.sh "pass"
./agentplay/continue_game.sh "play swamp"
```

Each `continue_game.sh` call:
- Appends the choice to `agentplay/choices.txt`
- Resumes from the snapshot with all choices so far
- Plays ONE more choice
- Shows the NEXT available choices
- Saves an updated snapshot

### Rich Text Commands

You can use either numeric indices OR descriptive commands:

```bash
# Numeric (simple but fragile to menu changes)
./agentplay/continue_game.sh "0"

# Rich text (robust to option ordering)
./agentplay/continue_game.sh "play mountain"
./agentplay/continue_game.sh "cast lightning bolt"
./agentplay/continue_game.sh "target bob"
./agentplay/continue_game.sh "pass"
```

### Building a Reproducer

The agentplay workflow automatically builds reproducers:

1. Start a game session with `start_game.sh`
2. Add choices with `continue_game.sh` until you reach the bug
3. Copy the REPRODUCER command from the output
4. That command replays the entire sequence deterministically

Example reproducer from the output:
```bash
mtg resume "/path/to/agentplay/game.snapshot" \
    --override-p1=fixed --override-p2=fixed \
    --p1-fixed-inputs="0;1;pass;play swamp" \
    --p2-fixed-inputs="0;1;pass;play swamp" \
    --stop-on-choice="5" --json --log-tail=100
```

### Session Management

```bash
# Clean up and start fresh
rm -f agentplay/game.snapshot agentplay/choices.txt

# Or just run start_game.sh again (it cleans up automatically)
./agentplay/start_game.sh decks/new_deck.dck
```

## Advanced: Direct mtg tui Usage

If you need more control than the agentplay scripts provide, you can use `mtg tui` directly. Read the agentplay script contents and `mtg tui --help` for details.

### Manual Fixed Input Workflow

```bash
# Start with empty inputs and stop when exhausted
cargo run --release --bin mtg -- tui decks/grizzly_bears.dck decks/royal_assassin.dck \
    --seed=100 \
    --stop-when-fixed-exhausted \
    --p1=fixed --p1-fixed-inputs="" \
    --p2=fixed --p2-fixed-inputs="" \
    --log-tail=100

# You'll see available choices printed
# Add choices one at a time and re-run:
cargo run --release --bin mtg -- tui decks/grizzly_bears.dck decks/royal_assassin.dck \
    --seed=100 \
    --stop-when-fixed-exhausted \
    --p1=fixed --p1-fixed-inputs="0" \
    --p2=fixed --p2-fixed-inputs="0" \
    --log-tail=100
```

### Snapshot/Resume Workflow

```bash
# Save snapshot after N choices
cargo run --release --bin mtg -- tui DECK1.dck DECK2.dck \
    --seed=100 \
    --stop-on-choice=10 \
    --snapshot-output=game.snapshot \
    --p1=heuristic --p2=heuristic

# Resume with different controllers
cargo run --release --bin mtg -- resume game.snapshot \
    --override-p1=fixed --p1-fixed-inputs="0;1;2" \
    --override-p2=fixed --p2-fixed-inputs="0;1;2"
```

### Using --stop-on-choice

Stop after a specific number of total choices (both players):

```bash
# Stop after first choice
--stop-on-choice=1

# Stop after 10 choices
--stop-on-choice=10

# Stop after 5 choices by P1 only
--stop-on-choice=5:p1

# Stop after 3 choices by P2 only
--stop-on-choice=3:p2
```

### Controlled Initial Hands

```bash
# Set specific starting hands (1-7 cards, semicolon-separated)
--p1-draw="Mountain;Lightning Bolt;Mountain"
--p2-draw="Island;Counterspell;Island;Island"
```

### Starting from Puzzle States

```bash
# Load a pre-configured board state
--start-state="puzzles/combat_test.pzl"
```

## Key Concepts for AI Agents

**You cannot interact via stdin/TUI directly**, but you CAN:
- Use fixed input controllers with predetermined choices
- Use snapshot/resume to build up game states incrementally
- Use the agentplay scripts to manage this workflow automatically

**Fixed inputs** are semicolon-separated (`;`), not commas or spaces:
- Numeric: `--p1-fixed-inputs="0;1;2;0"`
- Rich text: `--p1-fixed-inputs="play mountain;cast bolt;target bob"`

**Both players share the same choice sequence** in agentplay mode:
- The game engine alternates between players as needed
- You don't specify which player - the game state determines it

**Snapshots preserve everything**:
- Full game state (cards, zones, life totals)
- RNG state (for deterministic replay)
- Controller state (for fixed/random controllers)
- Turn number and choice counters

**REPRODUCER commands** are automatically printed by the agentplay scripts:
- Copy-paste ready for bug reports
- Include all choices made so far
- Deterministic (same seed, same choices = same outcome)

## Tips for Building Good Reproducers

1. **Start minimal**: Use simple decks when possible
2. **Use controlled hands**: `--p1-draw` and `--p2-draw` to set up specific scenarios
3. **Document the bug**: Include the REPRODUCER command in issue reports
4. **Test determinism**: Run the reproducer multiple times to confirm it's consistent
5. **Keep it short**: Stop as soon as you see the bug - don't continue playing

## Reference

For all available options and flags:
```bash
cargo run --bin mtg -- tui --help
cargo run --bin mtg -- resume --help
```

For implementation details, read:
- `agentplay/start_game.sh` - Initialization workflow
- `agentplay/continue_game.sh` - Incremental choice workflow
- `src/main.rs` - Full CLI argument parsing
