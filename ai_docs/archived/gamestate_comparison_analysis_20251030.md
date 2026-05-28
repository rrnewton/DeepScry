# GameState Comparison Analysis for Snapshot/Resume Determinism

**Date**: 2025-10-28
**Git Depth**: #167 (706cc03)

## Summary

**OUTDATED**: This analysis from 2025-10-28 is NO LONGER ACCURATE. Recent fixes revealed that log differences are NOT cosmetic - they indicate real determinism bugs.

**Current Status (2025-10-30)**: 4/6 stress tests passing. Remaining failures have log divergence indicating actual non-determinism, not just "metadata differences".

## Investigation

### Test Method

1. Run game normally with `--save-final-gamestate` to save final state
2. Run same game stop-and-go (snapshot → resume) and save final state
3. Deep compare the two JSON gamestate files

### Findings

**Log Comparison**: ✅ **PERFECT MATCH**
- All game actions (draws, plays, attacks, damage, etc.) match exactly
- 6/6 test cases passing (Royal Assassin, White Aggro, Grizzly Bears)
- Both heuristic and random controller modes work

**GameState JSON Comparison**: ⚠️ **222 differences found**

All 222 differences are in: `undo_log.actions[N].ChoicePoint.choice_id`

Example:
```
undo_log.actions[32].ChoicePoint.choice_id: 2 != 1
undo_log.actions[47].ChoicePoint.choice_id: 3 != 2
undo_log.actions[49].ChoicePoint.choice_id: 4 != 3
...
```

### Root Cause Analysis

The `choice_id` is a **monotonic counter** in `GameState` that increments each time a ChoicePoint is created. This counter is used for undo log tracking, not for gameplay logic.

**Normal game**: choice_id sequence is 1, 2, 3, 4, 5, ...

**Stop-and-go game**: choice_id starts at 2 instead of 1
- Why? The snapshot itself logs a ChoicePoint when checking the stop condition
- When we resume, the counter has already been incremented once
- So the first "real" choice after resume gets ID=2 instead of ID=1
- This offset propagates through the rest of the game: (2, 3, 4, 5, 6, ...)

### Impact Assessment

**Gameplay Determinism**: ✅ **PERFECT**
- All game actions match exactly (verified by log comparison)
- Same winners, same life totals, same turn counts
- Controller choices match (verified by stress tests)
- Card draws, combat, damage all identical

**Undo Log Metadata**: ⚠️ **ANALYSIS WAS WRONG**
- Previous analysis claimed choice_id differences were "cosmetic"
- This was incorrect - subsequent investigation found real determinism bugs
- Log differences indicate actual execution divergence, not just metadata
- The deterministic action log is the source of truth, not gamest state JSON

### Conclusion (REVISED)

**The original conclusion was WRONG**. Log differences are NOT "cosmetic" - they indicate broken determinism. The deterministic action log (stdout) is the canonical representation of game execution. Any log divergence means the games executed differently, even if gamestates happen to match by coincidence.

## Recommendations (REVISED)

### REJECT Previous "Option 1" - It Was Wrong

The previous recommendation to "accept the difference" was based on faulty analysis. DO NOT accept log differences as "cosmetic".

### Current Approach (CORRECT)

**Requirement**: Logs MUST match exactly for true determinism.
- The deterministic action log (stdout) is the canonical game record
- Any log divergence indicates broken determinism
- Fix all log divergence issues before declaring determinism achieved
- Gamestate JSON comparison is secondary to log comparison

### Option 2: Fix the Counter Offset

**If we want perfect JSON matching**, we could:

1. **Reset choice_id counter** when creating GameSnapshot for final state:
   ```rust
   // In --save-final-gamestate handler
   game.reset_choice_id_counter();  // Reset to 0 before snapshot
   let final_snapshot = GameSnapshot::new(game.clone(), ...);
   ```

2. **Don't log ChoicePoint** for stop condition checks:
   - Modify stop condition logic to not increment counter
   - Would require refactoring how stop conditions are checked

3. **Exclude choice_id from comparison**:
   - Modify Python comparison script to ignore `choice_id` fields
   - Still compare all actual gameplay state

**Recommended**: Option 1 (accept the difference) or Option 3 (exclude from comparison)

## Test Results

All determinism tests **PASS** with log comparison:

```
✓ Royal Assassin (heuristic vs heuristic): PASS
✓ Royal Assassin (random vs heuristic): PASS
✓ White Aggro 4ED (heuristic vs heuristic): PASS
✓ White Aggro 4ED (random vs heuristic): PASS
✓ Grizzly Bears (heuristic vs heuristic): PASS
✓ Grizzly Bears (random vs heuristic): PASS
```

## Appendix: Full Difference Pattern

All 222 differences follow the same pattern - choice_id in stop-and-go is offset by +1:

- Normal: 1, 2, 3, 4, 5, 6, ...
- Stop-go: 2, 3, 4, 5, 6, 7, ...

This affects every ChoicePoint in the undo log but does not affect:
- Actual choice values
- Game state (players, cards, zones, etc.)
- RNG state
- Controller state
- Turn counter
- Any gameplay logic

The offset is exactly 1 because exactly 1 ChoicePoint is logged during the snapshot creation process.
