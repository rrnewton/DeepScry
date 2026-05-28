# RNG State Serialization Issue

## Problem

**Date**: 2025-10-27
**Status**: CRITICAL BUG - blocks snapshot/resume determinism

### Current Architecture

```rust
GameState {
    rng_seed: u64  // Only stores INITIAL seed
}

RandomController {
    rng: Box<dyn RngCore>  // Evolves over time, NOT serialized
}
```

### The Bug

1. **Snapshot Creation**: GameState is serialized with `rng_seed` (initial seed only)
2. **RNG State Lost**: Controller's evolved RNG state is NOT saved
3. **Resume**: Controllers recreated with initial seed → RNG back to state 0
4. **Divergence**: Even with perfect choice replay, RNG state doesn't match

### Concrete Example

```
Normal Game:
- Turn 1-3: RandomController makes 50 RNG calls
- RNG state: advanced to position 50
- Saves snapshot with rng_seed = 42

Resume from Snapshot:
- New RandomController created with seed = 42
- RNG state: back to position 0 (not 50!)
- ReplayController replays 10 intra-turn choices
- Then delegates to RandomController with WRONG RNG state
- Future choices diverge
```

### Evidence

Stress test shows GameStates diverge with different card IDs selected:
```
✗ GameStates differ! Found 6 line differences
  Line 16:
    Normal:        25,
    Stop-go:       9,
```

## Solution

### Option A: Serialize RNG State in GameState (RECOMMENDED)

1. Enable `serde1` feature for `rand` crate
2. Add `rng: StdRng` to GameState (replace `rng_seed`)
3. Serialize the actual RNG state, not just seed
4. Controllers access RNG via GameStateView

**Pros**:
- Single source of truth for RNG state
- Automatically serialized with GameState
- Controllers can't get out of sync

**Cons**:
- Architectural change (RNG moves from controller to GameState)
- Requires updating all controller implementations

### Option B: Separate RNG Serialization

1. Add RNG state to GameSnapshot
2. Serialize each controller's RNG separately
3. Restore controllers with RNG state

**Pros**:
- Minimal architectural change
- Controllers keep their own RNG

**Cons**:
- More complex snapshot format
- Controllers must expose RNG state
- Multiple RNGs to track and synchronize

## Related Issues

- **mtg-100**: Separate seeds for initial shuffle vs gameplay RNG
  - Shuffle RNG: Used once at game start for deterministic library shuffling
  - Gameplay RNG: Used by controllers for ongoing decisions
  - These should be separate to allow independent seeding

## Implementation Notes

### Using StdRng with Serde

```toml
# Cargo.toml
rand = { version = "0.8", features = ["serde1"] }
```

```rust
use rand::SeedableRng;
use rand::rngs::StdRng;
use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize)]
struct GameState {
    // Replace this:
    // rng_seed: u64

    // With this:
    rng: StdRng,  // Fully serializable with current state
}
```

### Controller Access Pattern

```rust
impl PlayerController for RandomController {
    fn choose_spell_ability_to_play(
        &mut self,
        view: &GameStateView,
        available: &[SpellAbility],
    ) -> Option<SpellAbility> {
        // Get mutable access to GameState's RNG
        let rng = view.game_state_mut().get_rng_mut();
        let idx = rng.gen_range(0..available.len());
        Some(available[idx].clone())
    }
}
```

## Next Steps

1. Decide between Option A (move RNG to GameState) vs Option B (serialize separately)
2. Update Cargo.toml to enable `serde1` feature for `rand`
3. Implement chosen solution
4. Update all controllers to use new RNG access pattern
5. Update snapshot serialization to include RNG state
6. Verify determinism tests pass

## Testing

After implementing, verify:
1. Snapshot stress tests pass (deterministic game states)
2. RNG state correctly restored across snapshot/resume
3. No divergence in card IDs between normal and stop-go runs
4. Initial shuffle determinism preserved
