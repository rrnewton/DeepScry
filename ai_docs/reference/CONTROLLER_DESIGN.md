# Controller Architecture

## Overview

The `PlayerController` trait defines the interface for AI and UI implementations to make game decisions. This design closely matches Java Forge's PlayerController.java, providing a unified interface for all player decisions during gameplay.

## PlayerController Trait

Located in `src/game/controller.rs`, the trait provides methods for all decision points during an MTG game:

```rust
pub trait PlayerController {
    fn player_id(&self) -> PlayerId;

    // Main priority decision
    fn choose_spell_ability_to_play(
        &mut self,
        view: &GameStateView,
        available: &[SpellAbility]
    ) -> Option<SpellAbility>;

    // Spell casting decisions
    fn choose_targets(&mut self, view: &GameStateView, spell: CardId, valid_targets: &[CardId]) -> SmallVec<[CardId; 4]>;
    fn choose_mana_sources_to_pay(&mut self, view: &GameStateView, cost: &ManaCost, available_sources: &[CardId]) -> SmallVec<[CardId; 8]>;

    // Combat decisions
    fn choose_attackers(&mut self, view: &GameStateView, available_creatures: &[CardId]) -> SmallVec<[CardId; 8]>;
    fn choose_blockers(&mut self, view: &GameStateView, available_blockers: &[CardId], attackers: &[CardId]) -> SmallVec<[(CardId, CardId); 8]>;
    fn choose_damage_assignment_order(&mut self, view: &GameStateView, attacker: CardId, blockers: &[CardId]) -> SmallVec<[CardId; 4]>;

    // Other decisions
    fn choose_cards_to_discard(&mut self, view: &GameStateView, hand: &[CardId], count: usize) -> SmallVec<[CardId; 7]>;

    // Notifications
    fn on_priority_passed(&mut self, view: &GameStateView);
    fn on_game_ended(&mut self, view: &GameStateView, won: bool);
}
```

## Key Design Principles

### 1. Unified Spell Ability Selection

Instead of separate methods for lands, spells, and abilities, `choose_spell_ability_to_play()` returns any available action:
- Land plays (if can play lands this turn)
- Castable spells (if have mana and in appropriate phase)
- Activated abilities (if can activate)

This matches Java Forge's design where SpellAbility represents any playable action.

### 2. Correct Mana Timing

Mana is tapped during **step 6 of 8** in the casting process (MTG Rules 601.2g), AFTER the spell is on the stack. This is why `choose_mana_sources_to_pay()` is separate from `choose_spell_ability_to_play()`.

**Casting Process:**
1. Announce spell
2. Choose modes (if any)
3. **Choose targets** → `choose_targets()` called
4. Distribute effects
5. Check legality
6. **Determine total cost**
7. **Activate mana abilities** → `choose_mana_sources_to_pay()` called
8. Pay costs

### 3. GameStateView for Read-Only Access

Controllers receive a `GameStateView` that provides read-only access to game state:

```rust
pub struct GameStateView<'a> {
    game: &'a GameState,
    player_id: PlayerId,
}
```

**Available information:**
- `hand()` - Cards in this player's hand
- `battlefield()` - All cards on battlefield
- `graveyard()` - Cards in this player's graveyard
- `player_hand(player_id)` - Any player's hand
- `player_graveyard(player_id)` - Any player's graveyard
- `is_card_in_zone(card_id, zone)` - Check card location
- `get_card(card_id)` - Get card details
- `get_mana_pool(player_id)` - Check available mana
- `current_phase()` - Current game phase
- `current_step()` - Current game step
- `active_player()` - Whose turn it is

### 4. Zero-Copy Principles

All methods use:
- `&[CardId]` slices for input (no allocation)
- `SmallVec` for output (stack allocation for small collections)
- `&GameStateView` borrows (no cloning)

This maintains high performance even during tree search with millions of game states.

## Current Implementations

### 1. RandomController (`random_controller.rs`)
- Makes random decisions using a seeded RNG
- Used for testing and baseline performance
- Fully deterministic with same seed

### 2. ZeroController (`zero_controller.rs`)
- Always chooses the first available option
- Deterministic and predictable
- Used for testing

### 3. HeuristicController (`heuristic_controller.rs`)
- Evaluation-based AI ported from Java Forge
- Considers creature quality, removal priority, combat outcomes
- Most sophisticated AI currently available

### 4. FixedScriptController (`fixed_script_controller.rs`)
- Replays pre-recorded decisions from a script
- Used for determinism testing and replay functionality
- Verifies game state is reproducible

### 5. InteractiveController (`interactive_controller.rs`)
- Human player via stdin/stdout
- Provides text-based UI for testing
- Shows available options and accepts numeric choices

### 6. ReplayController (`replay_controller.rs`)
- Replays choices from a recorded game
- Used with snapshot/resume functionality
- Ensures identical gameplay when resuming from snapshots

## Java Forge Compatibility

This design closely matches Java Forge's architecture:

**Java Forge:**
```java
public interface PlayerController {
    SpellAbility chooseSpellAbilityToPlay();
    List<Card> chooseTargetsFor(SpellAbility sa);
    // ... other methods
}
```

**Rust Version:**
```rust
pub trait PlayerController {
    fn choose_spell_ability_to_play(...) -> Option<SpellAbility>;
    fn choose_targets(...) -> SmallVec<[CardId; 4]>;
    // ... other methods
}
```

**Key differences:**
- Rust uses `Option<T>` instead of null
- Rust uses `SmallVec` for efficiency instead of `ArrayList`
- Rust uses `&[T]` slices instead of `List<T>` for zero-copy
- Rust separates read-only view (GameStateView) from mutable GameState

## GameLoop Integration

The GameLoop (`game_loop.rs`) orchestrates the interaction:

1. **Detect decision point** (e.g., player has priority)
2. **Gather available options** (e.g., get castable spells from game state)
3. **Call controller** with options
4. **Execute chosen action** on game state

Example from priority handling:
```rust
// Gather available actions
let available_spells = self.get_available_spell_abilities(player_id);

// Ask controller
let choice = controller.choose_spell_ability_to_play(&view, &available_spells);

if let Some(ability) = choice {
    // Execute the chosen action
    self.execute_spell_ability(player_id, ability)?;
} else {
    // Pass priority
    controller.on_priority_passed(&view);
}
```

## Testing

Controller tests are in `controller_tests.rs` and include:
- Unit tests for each controller type
- Integration tests for full game scenarios
- Determinism tests (same seed → same outcome)
- Snapshot/resume tests with ReplayController

## Future Enhancements

- **MCTS/Minimax Controllers**: Tree search algorithms for stronger play
- **Neural Network Controllers**: ML-based decision making
- **Profile-Based Heuristics**: Different AI personalities (aggressive, control, etc.)
- **Learning Controllers**: Adapt strategy based on opponent behavior

## Summary

The PlayerController trait provides a clean, efficient interface for implementing game AI and UI. It closely matches Java Forge's proven design while leveraging Rust's zero-cost abstractions for better performance.
