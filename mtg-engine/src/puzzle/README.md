# Puzzle Module - PZL File Support

This module provides parsing and loading for .pzl (puzzle) files from Java Forge, allowing creation of specific mid-game states for testing and puzzle scenarios.

## Status: Parser and Loader Complete ✅

**What's Implemented:**
- ✅ Complete PZL file parser (format.rs, metadata.rs, state.rs, card_notation.rs)
- ✅ Metadata parsing (name, goal, difficulty, turns, descriptions)
- ✅ Game state parsing (turn, phase, player life, zones)
- ✅ Player state (life, lands played, counters, mana pool)
- ✅ All zone support (hand, battlefield, graveyard, library, exile)
- ✅ Card modifier parsing (tapped, counters, summoning sickness, etc.)
- ✅ Game state loader (loader.rs) - working with actual Card/Player/GameState structures
- ✅ 29 comprehensive unit tests (parser + loader)
- ✅ 2 working examples (load_puzzle.rs, load_mtgp05.rs)
- ✅ Support for 8 goal types
- ✅ Forward-compatible parser (ignores unknown fields)

**What's Working:**
- ✅ Loading simple puzzles with basic cards
- ✅ Applying player life totals
- ✅ Populating all zones (hand, battlefield, graveyard, library, exile)
- ✅ Setting turn number and phase
- ✅ Setting active player
- ✅ Applying tapped state to permanents
- ✅ Applying counters to cards
- ✅ Handling summoning sickness for creatures

## Usage

### Parsing a PZL File

```rust
use mtg_forge_rs::puzzle::PuzzleFile;
use std::path::Path;

// Load and parse a puzzle file
let puzzle = PuzzleFile::load(Path::new("test_puzzles/PP04.pzl"))?;

// Access metadata
println!("Puzzle: {}", puzzle.metadata.name);
println!("Goal: {:?}", puzzle.metadata.goal);
println!("Difficulty: {:?}", puzzle.metadata.difficulty);

// Access game state
println!("Turn: {}", puzzle.state.turn);
println!("Active player: {:?}", puzzle.state.active_player);
println!("P1 Life: {}", puzzle.state.players[0].life);
println!("P1 Hand: {} cards", puzzle.state.players[0].hand.len());
```

### PZL File Format

See `docs/PZL_FORMAT_ANALYSIS.md` for complete documentation.

Quick example:
```ini
[metadata]
Name:Test Puzzle
Goal:Win
Turns:1
Difficulty:Easy

[state]
turn=1
activeplayer=p0
activephase=MAIN1
p0life=20
p0hand=Lightning Bolt;Mountain
p0battlefield=Forest|Tapped
p1life=10
```

## Supported Card Modifiers (Phase 1)

- **Basic states**: `Tapped`, `SummonSick`, `FaceDown`, `Transformed`, `Flipped`
- **Counters**: `P1P1`, `M1M1`, `Loyalty`, `Poison`, `Energy`, `Charge`, `Age`, `Storage`
- **References**: `Id:123`, `AttachedTo:123`, `EnchantingPlayer:P0`
- **Damage**: `Damage:3`
- **Ownership**: `Owner:P0`
- **Combat**: `Attacking`, `Attacking:123`
- **Choices**: `ChosenColor`, `ChosenType`, `NamedCard`
- **Memory**: `RememberedCards`, `Imprinting`, `ExiledWith`
- **Special**: `IsCommander`, `IsRingBearer`, `NoETBTrigs`
- **Tokens**: `t:TokenName` (parsing only, application pending)

## Architecture

```
puzzle/
  mod.rs              - Public API and module exports
  format.rs           - INI-style section parser
  metadata.rs         - Metadata section (name, goal, difficulty)
  state.rs            - Game state section (turn, phase, players, zones)
  card_notation.rs    - Card modifier parsing (|Tapped|Counters:P1P1=3)
  loader.rs           - Apply parsed state to Game (IN PROGRESS)
  README.md           - This file
```

## Testing

```bash
# Run all puzzle module tests
cargo test --lib puzzle

# Run specific test module
cargo test --lib puzzle::format
cargo test --lib puzzle::metadata
cargo test --lib puzzle::state
cargo test --lib puzzle::card_notation

# All 27 tests should pass
```

## Next Steps (Future Enhancements)

1. **Advanced Card Modifiers**
   - Token creation and management
   - Card attachments (Auras, Equipment)
   - Enchanting players
   - Attacking/blocking assignments
   - Transform/flip states
   - Face-down cards (morph/manifest)

2. **Command Zone Support**
   - Add command zone to PlayerZones
   - Support for Commander/Oathbreaker puzzles

3. **Goal Enforcement**
   - Implement win condition checking
   - Track goal completion (DestroySpecifiedPermanents, etc.)
   - Timeout after turn limit
   - Automatic puzzle verification

4. **Combat State**
   - Restore attacking/blocking assignments
   - Apply combat damage modifiers
   - Handle first strike/double strike ordering

5. **More Example Puzzles**
   - Create example suite with known solutions
   - Automated verification of puzzle solutions
   - Performance benchmarks for puzzle loading

## Known Limitations

- ⚠️ Token creation not yet implemented
- ⚠️ Command zone not yet in PlayerZones (requires architecture change)
- ⚠️ Card attachments (Auras, Equipment) not yet applied
- ⚠️ Combat state (attacking/blocking) not restored
- ⚠️ Transform/flip/face-down states not yet applied
- ⚠️ Some cards may not be in cardsfolder (depends on Java Forge version)
- ⚠️ Mana pool persistence not implemented
- ⚠️ Player counters (poison, energy) not yet supported

These limitations don't prevent basic puzzle loading - they're advanced features
that can be added incrementally as needed.

## Examples

Real puzzle files can be found in:
- `forge-java/forge-gui/res/puzzle/*.pzl` (50+ examples)

Simple test cases are in the unit tests throughout this module.

## References

- Complete format documentation: `docs/PZL_FORMAT_ANALYSIS.md`
- Java implementation: `forge-java/forge-ai/src/main/java/forge/ai/GameState.java`
- Java loader: `forge-java/forge-gui/src/main/java/forge/gamemodes/puzzle/Puzzle.java`
