# Core Game Types and Entities

This module contains the fundamental types and entities for the MTG game engine. These are the building blocks used throughout the codebase.

## Module Overview

### [`entity.rs`](entity.rs)
**Purpose:** Generic entity system with strongly-typed IDs

- `EntityId<T>` - Type-safe entity identifier (zero-cost wrapper around u32)
- `EntityStore<T>` - Storage and management for entities
- `GameEntity` - Trait for types that can be stored as entities

**Key types:**
- `PlayerId = EntityId<Player>` - Strongly-typed player ID
- `CardId = EntityId<Card>` - Strongly-typed card ID

**Design principles:**
- Type safety: Can't accidentally use a CardId where PlayerId is expected
- Zero-cost: EntityId<T> compiles to a plain u32
- Copy semantics: IDs are small and cheap to copy

### [`card.rs`](card.rs)
**Purpose:** Card representation and properties

- `Card` - Runtime card instance with state (tapped, counters, etc.)
- `CardType` - Enum for card types (Creature, Instant, Sorcery, etc.)
- Power/toughness, keywords, abilities

**Notable:**
- Cards are stored in `EntityStore<Card>` and referenced by `CardId`
- Separates card definition (from loader) from runtime state
- Tracks summoning sickness, tap status, damage, counters

### [`player.rs`](player.rs)
**Purpose:** Player representation

- `Player` - Player state (life total, name, zones)
- Player-specific resources and stats

### [`mana.rs`](mana.rs)
**Purpose:** Mana system

- `Color` - Enum for mana colors (White, Blue, Black, Red, Green, Colorless)
- `ManaCost` - Represents a mana cost (e.g., {2}{U}{U})
- `ManaPool` - Tracks available mana for a player

**Key features:**
- Flexible mana cost representation
- Generic mana ({2} = any two mana)
- Colored mana requirements

### [`spell_ability.rs`](spell_ability.rs)
**Purpose:** Unified representation of playable actions

- `SpellAbility` - Represents any playable action:
  - Playing a land
  - Casting a spell
  - Activating an ability

**Design:** Matches Java Forge's SpellAbility concept where everything you can "do" is a spell ability.

### [`effects.rs`](effects.rs)
**Purpose:** Card effects and abilities

- `Effect` - Enum of all possible card effects:
  - `DealDamage` - Deal damage to target
  - `Draw` - Draw cards
  - `Destroy` - Destroy permanent
  - `GainLife` - Gain life
  - `Pump` - Modify power/toughness
  - `Tap`/`Untap` - Tap/untap permanents
  - `Mill` - Mill cards from library
  - `Counter` - Counter spell
  - `CreateToken` - Create token
  - `Regenerate` - Regenerate creature

- `Keyword` - Combat and static keywords:
  - Flying, First Strike, Double Strike, Deathtouch
  - Trample, Vigilance, Haste, Lifelink
  - Menace, Reach, Defender, Hexproof, Indestructible

- `Trigger` - Triggered abilities (ETB, dies, etc.)
- `TriggerEvent` - Events that can trigger abilities
- `ActivatedAbility` - Activated abilities with costs

### [`costs.rs`](costs.rs)
**Purpose:** Ability costs

- `Cost` - Enum of costs for activated abilities:
  - Mana costs
  - Tap costs
  - Sacrifice costs
  - Discard costs
  - etc.

### [`types.rs`](types.rs)
**Purpose:** String types and enums

- `CardName` - Efficient card name storage (Arc<str>)
- `PlayerName` - Efficient player name storage (Arc<str>)
- `Subtype` - Card subtypes (Goblin, Elf, etc.)
- `CounterType` - Counter types (+1/+1, -1/-1, etc.)

**Optimization:** Uses `Arc<str>` instead of `String` to avoid cloning when sharing names.

## Design Principles

### 1. Strong Typing
Uses newtype pattern extensively:
```rust
pub type PlayerId = EntityId<Player>;
pub type CardId = EntityId<Card>;
```
This prevents bugs like passing a CardId where PlayerId is expected.

### 2. Zero-Cost Abstractions
- `EntityId<T>` is a zero-cost wrapper (compiles to u32)
- Copy semantics for small types (IDs, enums)
- `Arc<str>` for shared immutable strings

### 3. Separation of Concerns
- **Core types** (this module) - Pure data structures
- **Game logic** (`src/game/`) - Game rules and state management
- **Card definitions** (`src/loader/`) - Loading cards from files

### 4. Java Forge Compatibility
Types are designed to match Java Forge concepts:
- `SpellAbility` ↔ Java's SpellAbility
- `Effect` ↔ Java's ApiType effects
- `Keyword` ↔ Java's Keyword enum
- `Card` ↔ Java's Card class

## Usage Examples

### Creating and Using Entity IDs
```rust
use crate::core::{PlayerId, CardId, EntityId};

let player1: PlayerId = EntityId::new(0);
let player2: PlayerId = EntityId::new(1);
let card: CardId = EntityId::new(42);

// Type safety: This won't compile!
// let wrong: PlayerId = card; // ERROR: type mismatch
```

### Working with Mana Costs
```rust
use crate::core::{ManaCost, Color};

// {2}{U}{U} - Two generic + two blue
let cost = ManaCost {
    generic: 2,
    white: 0,
    blue: 2,
    black: 0,
    red: 0,
    green: 0,
    colorless: 0,
};
```

### Defining Effects
```rust
use crate::core::Effect;

// Lightning Bolt: Deal 3 damage
let bolt_effect = Effect::DealDamage {
    amount: 3,
    target: target_id,
};

// Divination: Draw 2 cards
let draw_effect = Effect::Draw { count: 2 };
```

## Testing
Unit tests for core types are colocated in their respective modules.

## See Also
- `src/game/` - Game state and logic that uses these types
- `src/loader/` - Loading card definitions that create these types
- `ai_docs/CARD_SCRIPT_SPEC.md` - Card script format specification
