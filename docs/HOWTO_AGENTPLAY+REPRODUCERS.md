# HOWTO: Play MTG Games and Build Reproducers (For AI Agents)

It's important that you have the experience of playing the MTG game we're developing so you can see actual issues with gameplay and compare to expected behavior given MTG rules. Testing in code is insufficient.

## Quick Start: Using the agentplay Scripts (Recommended)

The easiest way to play games step-by-step and build reproducers is using the `agentplay/` wrapper scripts:

### Agent-Driven Bug Finding

```bash
# Bug-detection mode is on by default. Claude may choose a number or STOP with BUG_REPORT.
./agentplay/agent_game.py -- decks/simple_bolt.dck decks/simple_bolt.dck

# Keep a target scenario in context at every decision:
./agentplay/agent_game.py --seed 42 \
    --scenario "Play until P2 attacks with a flying creature, try to double-block with both creatures and cast an instant as a combat trick" \
    -- decks/booster_draft/avatar/eric_avatar_draft.dck decks/booster_draft/avatar/gabriel_avatar_draft.dck

# Disable STOP/BUG_REPORT prompting for pure gameplay:
./agentplay/agent_game.py --pure-play -- decks/simple_bolt.dck decks/simple_bolt.dck

# Mock mode uses local random choices and burns no agent tokens:
./agentplay/agent_game.py --mock --seed 42 -- decks/simple_bolt.dck decks/simple_bolt.dck
```

Each agent prompt includes the current state, the game log since the last
decision, the previous decision and rationale, the full interleaved history of
earlier log chunks plus choices, and the current choices. Use `--scenario` to
keep an English reproduction target in the prompt while the agent plays.

### Starting a New Game Session

```bash
# Start with any valid mtg tui arguments
./agentplay/start_game.py decks/simple_bolt.dck decks/simple_bolt.dck

# Or with specific initial hands:
./agentplay/start_game.py decks/grizzly_bears.dck decks/royal_assassin.dck \
    --p1-draw="Forest;Grizzly Bears;Forest"

# Or from a puzzle state:
./agentplay/start_game.py --start-state="puzzles/bolt_test.pzl"
```

This will:
1. Archive any existing `current.game` session to a numbered folder (001.game, 002.game, etc.)
2. Create a fresh `agentplay/current.game/` directory for this session
3. Initialize the game with deterministic seed (42)
4. Stop before the first choice is needed
5. Show you the available actions
6. Save session files to `agentplay/current.game/`
7. Print a REPRODUCER command for easy replay

### Adding Choices One at a Time

After start_game.py shows you the available choices, add them one at a time:

```bash
# Add a choice (specify player and choice)
./agentplay/continue_game.py p1 "0"

# The game will show the next available choices
# Continue adding choices as needed:
./agentplay/continue_game.py p1 "1"
./agentplay/continue_game.py p2 "pass"
./agentplay/continue_game.py p1 "play swamp"
```

Each `continue_game.py` call:
- Appends the choice to `agentplay/current.game/choices.txt`
- Replays the game from scratch with ALL choices accumulated so far
- Stops after the next choice is needed
- Shows the NEXT available choices
- Updates `agentplay/current.game/reproduce_game.sh` with the full reproducer

### Rich Text Commands

You can use either numeric indices OR descriptive commands:

```bash
# Numeric (simple but fragile to menu changes)
./agentplay/continue_game.py p1 "0"

# Rich text (robust to option ordering)
./agentplay/continue_game.py p1 "play mountain"
./agentplay/continue_game.py p1 "cast lightning bolt"
./agentplay/continue_game.py p1 "target bob"
./agentplay/continue_game.py p2 "pass"
```

For full syntax documentation including card name matching, wildcards, and special
cases, see [FIXED_INPUT_SYNTAX.md](./FIXED_INPUT_SYNTAX.md).

### Building a Reproducer

The agentplay workflow automatically builds reproducers:

1. Start a game session with `start_game.py`
2. Add choices with `continue_game.py` until you reach the bug
3. The reproducer is automatically saved to `agentplay/current.game/reproduce_game.sh`
4. Run that script to replay the entire sequence deterministically
5. Or copy the REPRODUCER command from the script output

Example reproducer from the output:
```bash
mtg tui decks/simple_bolt.dck decks/simple_bolt.dck \
    --p1=fixed --p2=fixed \
    --p1-fixed-inputs="0;1;pass;play swamp" \
    --p2-fixed-inputs="0;1;pass;play swamp" \
    --stop-on-choice=5 \
    --seed=42 --json --log-tail=100
```

The reproducer script in `current.game/reproduce_game.sh` includes the full `cargo run` command and is ready to execute.

### Session Management

```bash
# Start a new game (automatically archives the current session)
./agentplay/start_game.py decks/new_deck.dck

# Access archived sessions
ls agentplay/*.game/  # Shows current.game, 001.game, 002.game, etc.

# Replay an archived session
./agentplay/001.game/reproduce_game.sh

# Clean up all sessions
rm -rf agentplay/*.game/
```

Session files are stored in `agentplay/current.game/`:
- `choices.txt` - All choices made so far (one per line)
- `game.snapshot` - Latest game state snapshot
- `reproduce_game.sh` - Executable reproducer script
- `initial_args.txt` - Original mtg tui arguments

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
- Saved to `current.game/reproduce_game.sh` as an executable script
- Also printed in the terminal output for easy copy-paste
- Include all choices made so far
- Replay from scratch using `mtg tui` (not snapshots)
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
- [FIXED_INPUT_SYNTAX.md](./FIXED_INPUT_SYNTAX.md) - Complete input command syntax reference
- `agentplay/start_game.py` - Initialization workflow
- `agentplay/continue_game.py` - Incremental choice workflow
- `src/main.rs` - Full CLI argument parsing
