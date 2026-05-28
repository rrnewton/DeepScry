# Snapshot/Resume Architecture

## Snapshot Point Principle

**Snapshots are taken BEFORE presenting a choice to the controller.**

This design ensures:
1. **Clean pause point**: External agents can review the game state up to this point
2. **Clear semantics**: The snapshot represents "everything that has happened"
3. **Simple resume logic**: When resuming, present the next choice fresh to the controller

## Implementation

### PREAMBLE Check (Before Choice)

All choice points check stop conditions BEFORE asking the controller:

```rust
// PREAMBLE: Check stop conditions BEFORE asking for choice
if let Some(result) = self.check_stop_conditions(controller, player_id)? {
    return Ok(Some(result));  // Snapshot and stop HERE
}

// Ask controller to choose
let choice = controller.choose_spell_ability_to_play(&view, &available);

// Log choice to undo log
self.log_choice_point(player_id, Some(replay_choice));
```

### Stop Condition Logic

When `--stop-every both:choice:K` is specified:

1. **Before presenting choice K+1**:
   - `count_filtered_choices()` returns K (K choices have been made)
   - PREAMBLE check: K >= K? **Yes, snapshot**
   - Snapshot includes K choices (all made/executed/logged)
   - Game pauses BEFORE presenting choice K+1

2. **When resuming**:
   - Replay K choices with suppressed logging (already logged in previous segments)
   - Clear replay mode
   - Present choice K+1 fresh to the controller

### Replay Mode

```rust
// Replay mode suppresses ALL logging because snapshots are taken BEFORE
// presenting choices. All choices in the snapshot were already made/executed/logged.
if self.replaying {
    // Suppress stdout logging
}

// Clear replay mode before presenting the NEXT choice
if self.replaying && self.replay_choices_remaining == 0 {
    self.replaying = false;
}
```

## Example Timeline

**Segment 1** (stop after 3 choices):
1. Turn 1: Alice plays Forest → logged, executed
2. Turn 2: Bob plays Forest → logged, executed
3. Turn 3: Alice plays Forest → logged, executed
4. **PREAMBLE check**: count=3, 3 >= 3? Yes
5. **Snapshot**: Turn 3 start + 3 intra-turn choices
6. Stop (BEFORE presenting "Alice casts Grizzly Bears")

**Segment 2** (resume from snapshot):
1. Load: Turn 3 start, replay_mode(3)
2. Replay Alice plays Forest (Turn 1) - suppressed logging
3. Replay Bob plays Forest (Turn 2) - suppressed logging
4. Replay Alice plays Forest (Turn 3) - suppressed logging
5. Clear replay mode
6. **Present NEW choice**: "Alice casts Grizzly Bears" ← Fresh controller decision

## Benefits

1. **External agent workflow**: Can pause, review game log/state, then decide next move
2. **No ambiguity**: Snapshot never contains "partially made" choices
3. **Clean determinism**: All choices in snapshot were completed in previous segments
4. **Simple testing**: Can verify game state at natural pause points

## Comparison to Old Design

**Old (POSTAMBLE)**:
- Snapshot taken AFTER making choice K but BEFORE executing it
- Choice K in snapshot but never executed/logged to stdout
- Ambiguous: "Was this choice made or not?"
- Complex replay logic: Last choice needs special handling

**New (PREAMBLE)**:
- Snapshot taken BEFORE presenting choice K+1
- All K choices in snapshot were fully made/executed/logged
- Clear: "These choices are done, next one is pending"
- Simple replay logic: Suppress all, then present new choice

## Related Files

- `src/game/game_loop.rs`: Main implementation
  - `check_stop_conditions()`: PREAMBLE check before asking controller
  - `with_replay_mode()`: Sets up replay mode when resuming
  - `log_choice_point()`: Decrements replay counter
- `src/game/snapshot.rs`: GameSnapshot serialization
- `src/undo.rs`: `rewind_to_turn_start()` collects intra-turn choices
