# Game Engine

This module contains the game engine that implements MTG rules and manages game state. It orchestrates gameplay, handles player decisions, and enforces game rules.

## Module Overview

### Core Game State

#### [`state.rs`](state.rs)
**Purpose:** Central game state management

- `GameState` - Complete game state (players, cards, zones, stack, etc.)
- Zone management (battlefield, hand, library, graveyard, etc.)
- Card movement between zones
- Game state queries and modifications

**Key responsibilities:**
- Maintains all game entities (cards, players)
- Manages all zones (battlefield, stack, graveyards, hands, libraries)
- Provides API for game actions (play land, cast spell, etc.)
- Integrates with undo log for rewind functionality

#### [`game_loop.rs`](game_loop.rs)
**Purpose:** Main game loop and rules engine

- `GameLoop` - Orchestrates game flow
- Turn structure (untap, upkeep, draw, main, combat, end)
- Priority system
- Stack resolution
- Win condition checking

**Game flow:**
1. Turn phases and steps (MTG Rules 500-514)
2. Priority passing (MTG Rules 117)
3. Stack resolution (MTG Rules 608)
4. State-based actions (MTG Rules 704)

#### [`phase.rs`](phase.rs)
**Purpose:** Turn structure

- `Phase` - Enum for game phases (Beginning, Main1, Combat, Main2, End)
- `Step` - Enum for steps within phases (Untap, Upkeep, Draw, etc.)

### Player Controllers

#### [`controller.rs`](controller.rs)
**Purpose:** Player decision interface

- `PlayerController` trait - Interface for AI and UI
- `GameStateView` - Read-only game state for controllers
- All decision points (spell casting, combat, targeting, etc.)

**Controller methods:**
- `choose_spell_ability_to_play()` - Main priority decision
- `choose_targets()` - Target selection
- `choose_mana_sources_to_pay()` - Mana payment
- `choose_attackers()` / `choose_blockers()` - Combat decisions
- `choose_cards_to_discard()` - Hand size management

**See also:** `ai_docs/CONTROLLER_DESIGN.md` for detailed architecture

#### Controller Implementations

##### [`random_controller.rs`](random_controller.rs)
- Makes random decisions with seeded RNG
- Fully deterministic with same seed
- Used for testing and baseline performance

##### [`heuristic_controller.rs`](heuristic_controller.rs)
- Evaluation-based AI ported from Java Forge
- Creature quality evaluation
- Combat simulation
- Removal and threat assessment
- Most sophisticated AI currently available

##### [`interactive_controller.rs`](interactive_controller.rs)
- Human player via stdin/stdout
- Text-based UI for testing
- Shows available options and accepts numeric choices

##### [`fixed_script_controller.rs`](fixed_script_controller.rs)
- Replays pre-recorded decisions
- Used for determinism testing
- Verifies game state reproducibility

##### [`replay_controller.rs`](replay_controller.rs)
- Replays choices from recorded game
- Works with snapshot/resume functionality
- Ensures identical gameplay when resuming

##### [`controller_tests.rs`](controller_tests.rs)
- Unit and integration tests for controllers
- Determinism verification
- Snapshot/resume testing

### Game Actions and Effects

#### [`actions.rs`](actions.rs)
**Purpose:** High-level game actions

- Playing lands
- Casting spells
- Activating abilities
- Resolving effects
- Combat actions

#### [`combat.rs`](combat.rs)
**Purpose:** Combat phase implementation

- Declare attackers step
- Declare blockers step
- Combat damage calculation
- Damage assignment order
- First strike / double strike handling

#### [`mana_engine.rs`](mana_engine.rs)
**Purpose:** Mana management

- Mana pool tracking
- Paying mana costs
- Mana ability activation
- Generic vs. colored mana handling

### Evaluation and AI

#### [`game_state_evaluator.rs`](game_state_evaluator.rs)
**Purpose:** Board state evaluation for AI

- Score game states for decision making
- Creature evaluation (power, toughness, keywords)
- Life total weighting
- Card advantage calculation

Used by `HeuristicController` to make informed decisions.

### Snapshot and Replay

#### [`snapshot.rs`](snapshot.rs)
**Purpose:** Game state serialization

- `GameSnapshot` - Serializable game state
- Save/load game state to files
- Snapshot/resume functionality
- Used for tree search and game replay

**Features:**
- Binary serialization with serde
- Compact representation
- Fast save/load (critical for tree search)

#### [`stop_condition.rs`](stop_condition.rs)
**Purpose:** Configurable game stopping

- Stop after N turns
- Stop after N player choices
- Used with snapshot functionality
- Enables testing of snapshot/resume

### Logging and Debugging

#### [`logger.rs`](logger.rs)
**Purpose:** Game event logging

- `Logger` - Conditional logging system
- Verbosity levels (Silent, Normal, Verbose, Debug)
- Game action logging
- Replay analysis

**Features:**
- Zero-cost when disabled
- Structured event logging
- Helps with debugging and testing

#### [`logger_old.rs`](logger_old.rs)
**Purpose:** Legacy logging implementation (being phased out)

## Architecture

### Game Flow

```
GameLoop::run_game()
├─> Turn phases (untap, upkeep, draw, main, combat, end)
│   └─> For each step:
│       ├─> Execute step-based actions
│       └─> Handle priority
│           ├─> Active player gets priority
│           ├─> Controller chooses action
│           │   └─> PlayerController::choose_spell_ability_to_play()
│           ├─> Execute chosen action (or pass)
│           └─> Pass priority to next player
│
├─> Stack resolution
│   └─> Resolve top spell/ability
│       ├─> Choose targets (if needed)
│       ├─> Pay costs
│       └─> Apply effects
│
└─> Check win conditions
    └─> Return GameResult
```

### Controller Integration

```
┌─────────────┐
│  GameLoop   │
└──────┬──────┘
       │
       │ 1. Gather available actions
       │ 2. Create GameStateView
       │ 3. Call controller method
       ▼
┌──────────────────┐
│ PlayerController │ (trait)
└──────────────────┘
       │
       ├─> RandomController
       ├─> HeuristicController
       ├─> InteractiveController
       ├─> FixedScriptController
       └─> ReplayController
```

### State Management

```
┌─────────────┐
│  GameState  │
├─────────────┤
│ • players   │ EntityStore<Player>
│ • cards     │ EntityStore<Card>
│ • zones     │ Per-player zones
│ • battlefield
│ • stack     │
│ • undo_log  │ For rewind
└─────────────┘
```

## Design Principles

### 1. Faithful MTG Rules Implementation
- Follows comprehensive rules closely
- Phase/step structure matches MTG Rules 500-514
- Priority system matches MTG Rules 117
- Combat rules match MTG Rules 506-510

### 2. Controller Abstraction
- Clean separation: game engine vs. decision making
- Easy to implement new AIs and UIs
- Matches Java Forge's PlayerController pattern

### 3. Zero-Copy Performance
- GameStateView provides borrowing (no cloning)
- SmallVec for small collections
- Efficient entity ID system

### 4. Determinism
- Seeded RNG for reproducibility
- Determinism tests verify same seed → same outcome
- Critical for debugging and testing

### 5. Rewind Capability
- Undo log tracks all state changes
- Enables tree search (MCTS, minimax)
- Fast rewind for exploring game trees

## Testing

### Test Categories
1. **Unit tests** - In individual module files
2. **Controller tests** - `controller_tests.rs`
3. **Determinism tests** - `tests/determinism_e2e.rs`
4. **Integration tests** - Full game scenarios

### Running Tests
```bash
cargo nextest run game::  # Run all game module tests
cargo test --test determinism_e2e  # Determinism tests
```

## Performance Characteristics

**Benchmarks (as of 2025-10-26):**
- Fresh mode: ~3,842 games/sec
- Snapshot mode: ~9,177 games/sec
- Rewind mode: ~332k rewinds/sec
- Actions per turn: ~16.56 average

See `OPTIMIZATION.md` for detailed analysis.

## Java Forge Compatibility

This module closely follows Java Forge's architecture:

| Rust | Java Forge |
|------|------------|
| `GameState` | `Game` class |
| `GameLoop` | `GameAction` orchestration |
| `PlayerController` trait | `PlayerController` interface |
| `combat.rs` | `Combat.java` |
| `Phase` / `Step` enums | `PhaseType` / Phase handling |

**Key differences:**
- Rust uses ownership/borrowing (no GC)
- Explicit undo log (vs. Java's copying)
- SmallVec and slices for performance
- Strongly-typed EntityId system

## See Also
- `src/core/` - Core types used by game engine
- `src/loader/` - Loading card definitions
- `ai_docs/CONTROLLER_DESIGN.md` - Controller architecture
- `OPTIMIZATION.md` - Performance optimization guide
- `PROJECT_VISION.md` - Overall project design
