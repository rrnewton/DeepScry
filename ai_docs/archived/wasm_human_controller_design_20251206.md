# WASM Human Controller Design

## Problem Statement

The current WASM TUI implementation uses a separate game loop (`run_one_turn()`) that bypasses
the standard `GameLoop` used by the native implementation. This is problematic because:

1. **MTG is complex** - Having multiple game loops guarantees bugs from divergent logic
2. **The GameLoop has many features** - Snapshot/resume, replay, stop conditions, choice tracking
3. **Human input cannot block in WASM** - JavaScript is single-threaded with event-driven I/O

## Design Constraints

- **MUST use the single GameLoop** - No separate game loop for WASM
- **MUST NOT make GameLoop async** - Would impact performance on the hot path (benchmarks)
- **MUST support pause/resume at arbitrary choice points** - Human needs time to think
- **MUST leverage existing determinism** - Engine produces identical results given same RNG seed

## Architecture Overview

The solution leverages the existing **snapshot + replay** mechanism:

### Core Insight

The game engine is **deterministic**: given the same starting state, RNG seed, and sequence of
choices, the engine will always reach the same state. This means we can:

1. Save state at turn boundaries (snapshot)
2. Record choices made during a turn (intra-turn choice log)
3. Replay from turn start to reach any intra-turn state (deterministic replay)

### Event-Driven Approach

Instead of blocking for human input, we use an **interrupt pattern**:

```
Human's turn starts
├─ GameLoop runs until human needs to make a choice
├─ At choice point, HumanController returns "NeedInput"
├─ GameLoop saves state: (turn_start_snapshot, choices_so_far)
├─ GameLoop returns control to JS event loop
├─ UI displays choices in Actions pane
├─ ... time passes, human thinks ...
├─ Human makes selection via keyboard/mouse
├─ Game resumes:
│   ├─ Restore from turn_start_snapshot
│   ├─ Replay choices_so_far using ReplayController
│   └─ Continue with human's new choice
└─ Repeat until turn ends
```

### Key Components

#### 1. `ChoiceResult::NeedInput` Variant (New)

Add a new variant to `ChoiceResult<T>` that signals the game loop should pause:

```rust
pub enum ChoiceResult<T> {
    Ok(T),
    UndoRequest(usize),
    ExitGame,
    Error(String),
    NeedInput(ChoiceContext),  // NEW: pause and wait for async input
}
```

The `ChoiceContext` contains everything needed to display the choice to the user
and resume later.

#### 2. `WasmHumanController`

A controller that always returns `NeedInput` when a choice is required:

```rust
struct WasmHumanController {
    player_id: PlayerId,
    /// Pending choice to return (set by UI event handler)
    pending_choice: Option<ReplayChoice>,
}

impl PlayerController for WasmHumanController {
    fn choose_spell_ability_to_play(...) -> ChoiceResult<Option<SpellAbility>> {
        if let Some(ReplayChoice::SpellAbility(choice)) = self.pending_choice.take() {
            ChoiceResult::Ok(choice)
        } else {
            // Package up what we need to display
            ChoiceResult::NeedInput(ChoiceContext::SpellAbility { available: ... })
        }
    }
}
```

#### 3. `GameLoop::run_until_input()` Method

A new method that runs until either:
- Game ends (returns `GameResult`)
- Human input is needed (returns suspended state)

```rust
pub enum GameLoopState {
    Complete(GameResult),
    AwaitingInput {
        context: ChoiceContext,
        turn_snapshot: GameSnapshot,
        intra_turn_choices: Vec<ReplayChoice>,
    }
}

impl GameLoop {
    /// Run game until completion or human input needed
    pub fn run_until_input(&mut self, ...) -> Result<GameLoopState> {
        // Standard game loop, but on NeedInput:
        // 1. Save turn-start snapshot if not already saved
        // 2. Save intra-turn choices made so far
        // 3. Return AwaitingInput
    }
}
```

#### 4. Resume Protocol

When user makes a choice:

```rust
// In WASM event handler:
fn on_user_choice(choice_idx: usize) {
    let choice = ReplayChoice::SpellAbility(...);

    // Resume: restore snapshot, replay choices, add new choice
    let snapshot = state.pending_snapshot.take();
    let mut choices = state.intra_turn_choices.clone();
    choices.push(choice);

    // Create ReplayController for human player
    let human = WasmHumanController::new(p1_id);
    let replay = ReplayController::new(p1_id, Box::new(human), choices);

    // Run from snapshot with replay
    let result = game_loop.resume_from_snapshot(snapshot, &mut replay, &mut ai_controller);

    // Handle result (Complete or AwaitingInput again)
}
```

## Alternative Considered: Async GameLoop

Making the entire GameLoop async would be cleaner but has major drawbacks:

1. **Performance overhead** - Async machinery on every call in hot path
2. **Viral async** - Would need to make all callers async
3. **Native complexity** - Native doesn't need async, would add unnecessary complexity

The interrupt pattern avoids these issues by keeping the core synchronous.

## Compatibility

This design maintains full compatibility with:
- Existing native TUI (no changes needed)
- Existing benchmarks (no async overhead)
- Existing snapshot/resume (just using it differently)
- AI vs AI WASM games (AI controllers never return NeedInput)

## Implementation Steps

1. Add `ChoiceResult::NeedInput(ChoiceContext)` variant
2. Add `ChoiceContext` enum for different choice types
3. Implement `WasmHumanController` that returns `NeedInput`
4. Add `GameLoop::run_until_input()` method
5. Modify WASM TUI to use interrupt pattern
6. Add choice display in Actions pane
7. Wire up keyboard/mouse to resume with chosen option

## Open Questions

1. **Granularity of snapshots**: Should we snapshot at turn boundaries only, or could we
   be smarter (e.g., only when human player has priority)?

2. **Replay performance**: For long turns with many choices, replay could be slow.
   We might want to consider checkpointing within turns, but that adds complexity.

3. **Undo in browser**: The existing undo mechanism (`ChoiceResult::UndoRequest`) could
   be supported by re-running from snapshot with one fewer choice.
