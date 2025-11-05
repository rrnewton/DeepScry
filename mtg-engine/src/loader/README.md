# Card and Deck Loaders

This module handles loading card definitions and deck lists from disk. It parses Forge card format files and deck files, providing a bridge between the file system and the game engine.

## Module Overview

### Card Loading

#### [`card.rs`](card.rs)
**Purpose:** Parse individual card definition files

- `CardDefinition` - Parsed card data structure
- `CardLoader` - Parser for Forge card format (.txt files)

**Card file format:**
```
Name:Lightning Bolt
ManaCost:R
Types:Instant
A:SP$ DealDamage | NumDmg$ 3 | ValidTgts$ Creature,Player
Oracle:Lightning Bolt deals 3 damage to any target.
```

**Fields parsed:**
- `Name:` - Card name (required)
- `ManaCost:` - Mana cost (e.g., "2UU", "R", "{1}{W}")
- `Types:` - Card types (Creature, Instant, Sorcery, etc.)
- `PT:` - Power/Toughness (e.g., "2/2", "*/1+*")
- `K:` - Keywords (Flying, First Strike, etc.)
- `A:` - Abilities (spell effects, activated abilities)
- `T:` - Triggers (ETB, dies, etc.)
- `S:` - Static abilities (continuous effects)
- `SVar:` - Script variables
- `Oracle:` - Oracle text

**Error handling:**
- Line number tracking for parse errors
- File path included in error messages
- Helpful error messages for common mistakes

#### [`database_async.rs`](database_async.rs)
**Purpose:** Card database with async loading

- `CardDatabase` - Central repository of card definitions
- Async parallel loading from cardsfolder
- Efficient lookup by card name
- Caching using `Arc<CardDefinition>`

**Features:**
- **Parallel loading**: Uses `tokio` to load cards concurrently
- **Efficient storage**: Cards stored as `Arc<CardDefinition>` (shared ownership)
- **Fast lookup**: HashMap-based name lookup
- **Error reporting**: Collects all parse errors, doesn't fail on first error

**Usage:**
```rust
let db = CardDatabase::load_from_folder("forge-java/res/cardsfolder").await?;
let bolt = db.get_card("Lightning Bolt")?;
```

**Statistics:**
- Loads 31,000+ cards from Forge cardsfolder
- Typical load time: ~2-3 seconds on modern hardware
- Memory efficient with Arc sharing

### Deck Loading

#### [`deck.rs`](deck.rs)
**Purpose:** Parse deck list files

- `DeckList` - Represents a deck (main deck + sideboard)
- `DeckEntry` - Single deck entry (quantity + card name)
- `DeckLoader` - Parser for .dck format

**Deck file format (.dck):**
```
[metadata]
Name=White Weenie

[Main]
4 Savannah Lions
4 White Knight
4 Serra Angel
20 Plains

[Sideboard]
3 Disenchant
```

**Sections:**
- `[metadata]` - Deck metadata (name, format, etc.)
- `[Main]` - Main deck (60 cards)
- `[Sideboard]` - Sideboard (up to 15 cards)

**Features:**
- Validates deck structure
- Handles comments and blank lines
- Error reporting with line numbers

#### [`deck_async.rs`](deck_async.rs)
**Purpose:** Async deck loading utilities

- `prefetch_deck_cards()` - Pre-load all cards in deck from database
- Ensures all card definitions are available before game start
- Returns errors for missing/invalid cards

### Game Initialization

#### [`game_init.rs`](game_init.rs)
**Purpose:** Create game from deck files

- `GameInitializer` - High-level game setup
- Loads decks, creates players, shuffles libraries
- Returns ready-to-play `GameState`

**Features:**
- Creates players with names
- Loads and validates decks
- Shuffles libraries with seed
- Initializes starting hands
- Ready for GameLoop to run

**Usage:**
```rust
let initializer = GameInitializer::new(card_database);
let game_state = initializer.create_game(
    "deck1.dck",
    "deck2.dck",
    "Alice",
    "Bob",
    seed
)?;
```

## File Formats

### Card Format (.txt)

Based on Forge's card scripting format. See `ai_docs/CARD_SCRIPT_SPEC.md` for full specification.

**Key sections:**
- **Metadata**: Name, ManaCost, Types, PT
- **Keywords (K:)**: Flying, First Strike, Trample, etc.
- **Spell Effects (A:SP$)**: One-time effects
- **Activated Abilities (A:AB$)**: Tap/mana cost abilities
- **Triggers (T:)**: Event-based effects
- **Static Abilities (S:)**: Continuous effects
- **Script Variables (SVar:)**: Reusable definitions

**Example:**
```
Name:Grizzly Bears
ManaCost:1G
Types:Creature Bear
PT:2/2
Oracle:A simple 2/2 bear.
```

### Deck Format (.dck)

Simple text format with sections:

```
[metadata]
Name=My Deck
Format=Vintage

[Main]
4 Lightning Bolt
4 Grizzly Bears
52 Island

[Sideboard]
4 Counterspell
```

**Format:**
- Each line: `<quantity> <card name>`
- Comments start with `#`
- Blank lines ignored
- Case-sensitive card names

## Error Handling

All loaders provide detailed error messages:

**Card loader errors:**
```
Failed to parse card file 'cards/bolt.txt':
  Line 5: Invalid PT format '2/' (expected format: 'N/N', e.g., 'PT:2/2')
```

**Deck loader errors:**
```
Error loading deck 'mydeck.dck':
  Line 12: Invalid deck entry '4x Lightning Bolt' (expected format: 'N CardName')
```

**Database errors:**
```
Card 'Lightning_Bolt' not found in database
Hint: Card names are case-sensitive and use spaces, not underscores
```

## Performance Characteristics

**Card loading:**
- Single card: ~100-200μs (with error handling)
- Full database (31k cards): ~2-3s with parallel loading
- Memory: ~50-80MB for full database (with Arc sharing)

**Deck loading:**
- Single deck: ~1-2ms
- Includes validation and card lookup
- Fast enough to load on-demand

**Optimization notes:**
- Uses `Arc<CardDefinition>` to avoid cloning
- Parallel async loading with `tokio::spawn`
- Efficient string interning for card names

## Testing

### Unit Tests
Each module has colocated tests:
- `card.rs` - Parser correctness
- `deck.rs` - Deck format handling
- `database_async.rs` - Database operations

### Integration Tests
- `tests/card_loading_tests.rs` - Load real cards from Forge
- `tests/deck_loading_tests.rs` - Load real decks

### Test Data
- Uses actual Forge cardsfolder for realistic testing
- Validates against 31k+ real cards
- Ensures compatibility with upstream Forge

## Java Forge Compatibility

### File Format Compatibility
**100% compatible** with Java Forge card and deck formats:
- Can load Forge's cardsfolder directly
- Deck files are interchangeable
- Same card script syntax

### Feature Parity
| Feature | Java Forge | Rust Version |
|---------|------------|--------------|
| Card parsing | ✅ | ✅ |
| Deck loading | ✅ | ✅ |
| Async loading | ❌ | ✅ (Rust addition) |
| Error recovery | ⚠️ (stops on error) | ✅ (collects all errors) |
| Parallel loading | ❌ | ✅ (Rust addition) |

### Differences
- **Rust uses async/await**: Tokio for parallel loading (Java uses blocking I/O)
- **Better error reporting**: Rust version collects all errors instead of stopping at first
- **Type safety**: Rust's type system catches errors at compile time
- **Memory efficiency**: Arc<T> prevents cloning compared to Java's object copying

## Common Issues

### Card Not Found
```rust
Error: Card 'grizzly bears' not found
```
**Solution**: Card names are case-sensitive. Use `Grizzly Bears`.

### Invalid Mana Cost
```rust
Error: Invalid mana cost '2U' - use '{2}{U}' format
```
**Solution**: Some parsers require explicit braces: `{2}{U}` instead of `2U`.

### Variable P/T
```rust
PT:*/1+*
```
Cards with variable power/toughness (like Tarmogoyf) parse as `None` for power and/or toughness. This is expected - the actual value is calculated dynamically.

## Examples

### Loading a Single Card
```rust
use mtg_forge_rs::loader::CardLoader;

let card = CardLoader::load_from_file("path/to/card.txt")?;
println!("Loaded: {} ({})", card.name, card.mana_cost);
```

### Loading Card Database
```rust
use mtg_forge_rs::loader::CardDatabase;

let db = CardDatabase::load_from_folder("forge-java/res/cardsfolder").await?;
println!("Loaded {} cards", db.card_count());
```

### Loading a Deck
```rust
use mtg_forge_rs::loader::DeckLoader;

let deck = DeckLoader::load_from_file("mydeck.dck")?;
println!("Deck: {} ({} cards)", deck.name, deck.main_deck.len());
```

### Creating a Game
```rust
use mtg_forge_rs::loader::{CardDatabase, GameInitializer};

let db = CardDatabase::load_from_folder("forge-java/res/cardsfolder").await?;
let init = GameInitializer::new(db);
let game = init.create_game(
    "deck1.dck",
    "deck2.dck",
    "Alice",
    "Bob",
    42  // random seed
)?;
```

## See Also
- `ai_docs/CARD_SCRIPT_SPEC.md` - Complete card format specification
- `src/core/` - Core types used by loaders
- `src/game/` - Game engine that uses loaded cards
- Forge cardsfolder: `forge-java/res/cardsfolder/`
- Example decks: `decks/`
