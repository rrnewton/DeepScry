# Game Loop Refactoring Analysis

**File:** `/mtg-forge-rs-fedora/mtg-engine/src/game/game_loop.rs`
**Date:** 2025-11-25
**Total Size:** 3,455 lines (3,367 production + 88 tests)

## Executive Summary

The `game_loop.rs` file is a 3,455-line monolithic implementation managing turn progression, priority, combat, and game flow. Like the recent `actions.rs` refactoring (which split 6K lines into modular components), this file should be decomposed into focused submodules for maintainability and clarity.

**Key Metrics:**
- 70 total functions (66 production + 4 tests)
- ~97.4% production code, ~2.6% tests
- GameLoop struct has 17 fields (snapshot management, replay tracking, logging, etc.)
- Significant complexity in priority rounds (~544 lines), combat steps (~394 lines)

## Current File Structure

### Major Sections (Line Ranges)

| Section | Lines | Description |
|---------|-------|-------------|
| **Preamble** | 1-43 | Imports, macros (log_if_verbose), legacy PlayerAction enum |
| **Enums** | 44-87 | VerbosityLevel, GameResult, GameEndReason |
| **GameLoop Struct** | 88-139 | 17 fields including game state, snapshot tracking, replay mode |
| **impl GameLoop** | 140-3365 | 66 methods (main implementation) |
| **Tests** | 3367-3455 | 4 basic unit tests |

### Function Categories (66 production methods)

#### 1. Builder Methods (12 functions, ~148 lines)
Configuration methods for GameLoop construction:
- `new()` - Constructor
- `with_max_turns()` - Set turn limit
- `with_snapshot_format()` - Serialization format
- `with_verbosity()` - Output level control
- `with_turn_counter()` - Resume from snapshot
- `with_choice_counter()` - Choice tracking
- `with_stop_when_fixed_exhausted()` - Auto-snapshot mode
- `with_stop_condition()` - Mid-turn exit points
- `with_baseline_choice_count()` - Snapshot baseline
- `with_p1_hand_setup()` - Test hand control
- `with_p2_hand_setup()` - Test hand control
- `with_replay_mode()` - Replay tracking
- `with_verbose()` - Deprecated, use with_verbosity

**Lines:** 143-317

#### 2. Game Loop Control (14 functions, ~675 lines)
Core game flow orchestration:
- `run_game()` - Main game loop
- `run_turns()` - Bounded turn execution
- `run_turn_once()` - Single turn with end-condition checks
- `run_turn()` - Turn execution through all phases
- `reset()` - Reset state for replay
- `setup_game()` - Initialize game (shuffle, draw hands)
- `log_choice_point()` - Track controller decisions
- `check_stop_conditions()` - Snapshot triggers (BEFORE choice)
- `count_filtered_choices()` - Choice counting for stop conditions
- `assert_valid_stopping_point()` - Validation for snapshots
- `save_snapshot_and_exit()` - Persist game state
- `notify_game_end()` - Controller notifications
- `reset_turn_state()` - Per-turn cleanup

**Lines:** 143-169, 318-946, 1346-1364
**Complexity:** High - orchestrates entire game flow

#### 3. Turn and Step Execution (7 functions, ~290 lines)
Phase/step handlers:
- `execute_step()` - Dispatch to step handlers
- `untap_step()` - Untap permanents
- `upkeep_step()` - Upkeep triggers + priority
- `draw_step()` - Draw card + priority
- `main_phase()` - Main phase priority
- `end_step()` - End step + priority
- `cleanup_step()` - Discard to hand size, remove damage, end-of-turn effects

**Lines:** 1365-1545, 1953-2095
**Pattern:** Most steps delegate to `priority_round()`

#### 4. Combat Steps (7 functions, ~394 lines)
Combat phase implementation:
- `begin_combat_step()` - Begin combat + priority
- `declare_attackers_step()` - Choose attackers (~117 lines)
- `declare_blockers_step()` - Choose blockers (~116 lines)
- `combat_damage_step()` - Apply damage + SBA
- `has_first_strike_combat()` - Check for first strike
- `log_combat_damage()` - Detailed damage logging
- `end_combat_step()` - End combat + priority

**Lines:** 1559-1952
**Complexity:** High - complex controller interactions, replay mode handling

#### 5. Priority and Stack (4 functions, ~727 lines)
Priority system and spell resolution:
- `priority_round()` - **544 lines!** - Main priority loop
- `resolve_top_spell_from_stack()` - Spell resolution
- `check_phase_triggers()` - Triggered abilities
- `spell_requires_stack_target()` - Counter-spell check

**Lines:** 1429-1475, 2096-2759, 3356-3365
**Biggest culprit:** `priority_round()` is massive and handles:
  - Spell ability collection
  - Controller choice requests
  - Replay mode logic
  - Stop condition checks
  - Pass/priority switching
  - Stack resolution

#### 6. Action Queries (7 functions, ~320 lines)
Available actions for controllers:
- `get_available_attacker_creatures()` - Can attack
- `get_available_blocker_creatures()` - Can block
- `get_current_attackers()` - Current attackers list
- `get_lands_in_hand()` - Playable lands
- `get_castable_spells()` - Spells with sufficient mana
- `get_activatable_abilities()` - Abilities player can use
- `get_available_spell_abilities()` - All spell abilities (lands + spells + abilities)

**Lines:** 2883-3202
**Pattern:** Query game state for controller decisions

#### 7. Logging and Display (9 functions, ~399 lines)
Output and formatting:
- `get_player_name()` - Player display name
- `step_name()` - Step display name
- `print_battlefield_state()` - Detailed game state (~104 lines)
- `print_step_header_if_needed()` - Lazy header printing
- `should_print_to_stdout()` - Output gate
- `log_normal()` - Normal-level logging
- `log_verbose()` - Verbose logging
- `log_minimal()` - Minimal logging
- `log_effect_execution()` - **214 lines!** - Effect logging with extensive formatting

**Lines:** 947-1345
**Biggest logging method:** `log_effect_execution()` handles all effect types

#### 8. Win Conditions (1 function, ~34 lines)
Game-ending checks:
- `check_win_condition()` - Player death, decking

**Lines:** 3322-3355

#### 9. Legacy v1 Actions (5 functions, ~242 lines)
**DEPRECATED** - Old controller interface:
- `get_available_attackers()` - Legacy attacker actions
- `get_available_blockers()` - Legacy blocker actions
- `get_available_actions()` - Legacy action list
- `execute_action()` - Legacy action execution
- `describe_action()` - Legacy action descriptions

**Lines:** 2760-3321
**Status:** Marked for removal, superseded by v2 SpellAbility interface

## Proposed Refactoring Plan

### Strategy: Module Extraction (Similar to actions.rs)

Follow the pattern from `actions.rs` refactoring:
- `mod.rs` - Main GameLoop struct + core orchestration
- Submodules for distinct responsibilities
- Keep tests in `tests/` subdirectory

### Recommended Module Structure

```
src/game/game_loop/
├── mod.rs              (~800 lines)  # GameLoop struct, run_game, run_turn, builder methods
├── steps.rs            (~400 lines)  # Turn/step execution (untap, upkeep, draw, main, cleanup)
├── combat.rs           (~450 lines)  # Combat steps (attackers, blockers, damage, first strike)
├── priority.rs         (~600 lines)  # Priority round, stack resolution, triggers
├── actions.rs          (~350 lines)  # Action queries (get_castable_spells, get_available_*)
├── snapshot.rs         (~300 lines)  # Snapshot save/load, stop conditions, replay mode
├── logging.rs          (~450 lines)  # All logging/display (print_battlefield, log_effect_execution)
├── legacy.rs           (~250 lines)  # Legacy v1 PlayerAction (mark deprecated, remove later)
└── tests/
    ├── mod.rs
    ├── game_loop_tests.rs
    ├── combat_tests.rs
    └── priority_tests.rs
```

### Detailed Module Breakdown

#### **mod.rs** (~800 lines)
**Purpose:** Core GameLoop struct, top-level orchestration, builder pattern

**Contents:**
- GameLoop struct definition (17 fields)
- Enums: VerbosityLevel, GameResult, GameEndReason
- Legacy PlayerAction enum (until removed)
- Builder methods (with_* chain)
- Core loop: `run_game()`, `run_turns()`, `run_turn_once()`, `run_turn()`
- Setup: `setup_game()`, `reset()`, `reset_turn_state()`
- Helpers: `get_player_name()`, `step_name()`
- Win conditions: `check_win_condition()`
- Dispatch: `execute_step()` (calls step handlers)

**Complexity:** Medium - orchestration only, delegates to submodules

---

#### **steps.rs** (~400 lines)
**Purpose:** Individual step handlers (non-combat)

**Contents:**
- `untap_step()` - Untap permanents
- `upkeep_step()` - Upkeep triggers + priority
- `draw_step()` - Draw card + priority
- `main_phase()` - Main phase priority
- `end_step()` - End step + priority
- `cleanup_step()` - Discard, damage removal, end-of-turn effects

**Pattern:** Most methods are thin wrappers calling `priority_round()` from priority.rs

**Imports from priority.rs:** `priority_round()`

---

#### **combat.rs** (~450 lines)
**Purpose:** Combat phase implementation

**Contents:**
- `begin_combat_step()` - Begin combat + priority
- `declare_attackers_step()` - Attacker selection (~117 lines)
  - Controller choice with replay support
  - Stop condition checks
  - Attacker declaration
- `declare_blockers_step()` - Blocker selection (~116 lines)
  - Defender choice with replay support
  - Block declarations
- `combat_damage_step()` - Damage application
- `has_first_strike_combat()` - First strike detection
- `log_combat_damage()` - Combat damage logging (~73 lines)
- `end_combat_step()` - End combat + priority

**Imports from priority.rs:** `priority_round()`
**Imports from snapshot.rs:** `check_stop_conditions()`

---

#### **priority.rs** (~600 lines)
**Purpose:** Priority system, stack resolution, triggers

**Contents:**
- `priority_round()` - **544 lines** - THE BIG ONE
  - Pass priority between players
  - Collect available spell abilities
  - Controller choice handling
  - Replay mode integration
  - Stop condition checks (via snapshot.rs)
  - Stack resolution loop
- `resolve_top_spell_from_stack()` - Spell resolution (~120 lines)
  - Target validation
  - Effect execution with logging
  - Zone transitions
- `check_phase_triggers()` - Triggered ability handling (~47 lines)
- `spell_requires_stack_target()` - Counterspell targeting check

**Imports from actions.rs:** `get_available_spell_abilities()`
**Imports from snapshot.rs:** `check_stop_conditions()`
**Imports from logging.rs:** `log_effect_execution()`

**Complexity:** Very High - This is the heart of the game engine

---

#### **actions.rs** (~350 lines)
**Purpose:** Query available actions for controllers

**Contents:**
- `get_available_attacker_creatures()` - Can attack (~36 lines)
- `get_available_blocker_creatures()` - Can block (~21 lines)
- `get_current_attackers()` - Current attackers
- `get_lands_in_hand()` - Playable lands (~17 lines)
- `get_castable_spells()` - Spells with mana (~74 lines)
  - Uses mana_engine for availability
  - Target validation
- `get_activatable_abilities()` - Usable abilities (~112 lines)
  - Cost checking
  - Target validation
- `get_available_spell_abilities()` - Combined list (~55 lines)
  - Merges lands, spells, abilities

**Pattern:** Read-only queries, no side effects

**Imports:** Uses `mana_engine` field from GameLoop

---

#### **snapshot.rs** (~300 lines)
**Purpose:** Save/load snapshots, replay mode, stop conditions

**Contents:**
- `log_choice_point()` - Log controller choices (~46 lines)
- `check_stop_conditions()` - Pre-choice snapshot check (~39 lines)
  - Fixed controller exhaustion
  - --stop-on-choice condition
- `count_filtered_choices()` - Choice filtering (~31 lines)
- `assert_valid_stopping_point()` - Validation (~54 lines)
- `save_snapshot_and_exit()` - Persist to disk (~95 lines)
  - Serialize game state
  - Save controller choices
  - Write metadata
- Builder integration:
  - `with_stop_when_fixed_exhausted()`
  - `with_stop_condition()`
  - `with_baseline_choice_count()`
  - `with_replay_mode()`

**Complexity:** Medium - Interacts with undo_log, controllers

---

#### **logging.rs** (~450 lines)
**Purpose:** All logging and display formatting

**Contents:**
- `print_battlefield_state()` - **104 lines** - Detailed game state
  - Player life, zones
  - Battlefield permanents (creatures, lands, artifacts)
  - P/T display with modifiers
- `log_effect_execution()` - **214 lines** - Effect logging
  - Handles all effect types
  - Replay mode suppression
  - Rich formatting
- `print_step_header_if_needed()` - Lazy header (~17 lines)
- `should_print_to_stdout()` - Output gate (~7 lines)
- `log_normal()` - Normal logging (~10 lines)
- `log_verbose()` - Verbose logging (~10 lines)
- `log_minimal()` - Minimal logging (~7 lines)
- Helper: `get_player_name()` (if moved here)
- Helper: `step_name()` (if moved here)

**Pattern:** Pure formatting, delegates to game.logger

**Imports:** None - leaf module

---

#### **legacy.rs** (~250 lines)
**Purpose:** Legacy v1 PlayerAction interface (DEPRECATED)

**Contents:**
- `PlayerAction` enum (keep here until removed)
- `get_available_attackers()` - v1 attacker actions (~25 lines)
- `get_available_blockers()` - v1 blocker actions (~37 lines)
- `get_available_actions()` - v1 action list (~61 lines)
- `execute_action()` - v1 execution (~61 lines)
- `describe_action()` - v1 descriptions (~58 lines)

**Status:** Mark entire module as `#[deprecated]`, remove in future commit

---

## Refactoring Steps (Proposed Execution Order)

### Phase 1: Extract Pure Read-Only Modules (Low Risk)
1. **logging.rs** - No dependencies, pure formatting
   - Move: `print_*`, `log_*`, `should_print_to_stdout()`
   - Test: Verify output unchanged
2. **actions.rs** - Read-only queries
   - Move: `get_available_*`, `get_castable_*`, `get_activatable_*`
   - Test: Controller integration tests

### Phase 2: Extract Independent Logic (Medium Risk)
3. **snapshot.rs** - Self-contained snapshot logic
   - Move: `log_choice_point()`, `check_stop_conditions()`, `save_snapshot_and_exit()`
   - Test: Snapshot/resume integration tests
4. **legacy.rs** - Deprecated code
   - Move: PlayerAction enum, v1 action methods
   - Mark module `#[deprecated]`
   - Test: Legacy tests (if any)

### Phase 3: Extract Game Flow Modules (Higher Risk)
5. **steps.rs** - Non-combat step handlers
   - Move: `untap_step()`, `upkeep_step()`, `draw_step()`, `main_phase()`, `end_step()`, `cleanup_step()`
   - Imports: `priority::priority_round()`
   - Test: Full game simulation
6. **combat.rs** - Combat phase
   - Move: All combat step handlers
   - Imports: `priority::priority_round()`, `snapshot::check_stop_conditions()`
   - Test: Combat-focused games

### Phase 4: Extract Core Engine (Highest Risk)
7. **priority.rs** - Priority system
   - Move: `priority_round()`, `resolve_top_spell_from_stack()`, `check_phase_triggers()`
   - Imports: `actions::get_available_spell_abilities()`, `logging::log_effect_execution()`
   - Test: **Full validation suite** - this is the riskiest module

### Phase 5: Consolidate
8. **mod.rs** - Final cleanup
   - Keep: GameLoop struct, builder methods, core loop, setup
   - Re-export: `pub use steps::*;` etc.
   - Test: **Full regression test suite**

## Expected Line Counts After Refactoring

| Module | Lines | Complexity |
|--------|-------|------------|
| mod.rs | ~800 | Medium |
| steps.rs | ~400 | Low |
| combat.rs | ~450 | Medium |
| priority.rs | ~600 | **Very High** |
| actions.rs | ~350 | Low |
| snapshot.rs | ~300 | Medium |
| logging.rs | ~450 | Low |
| legacy.rs | ~250 | Low (deprecated) |
| **Total** | **~3,600** | - |

**Note:** Total slightly higher due to:
- Module declarations
- Re-exports in mod.rs
- Duplicate imports across modules

## Comparison to actions.rs Refactoring

### actions.rs Split (Recent)
- **Before:** 6,000 lines in single file
- **After:**
  - `mod.rs` (~70KB)
  - `combat.rs` (~23KB)
  - `targeting.rs` (~15KB)
- **Pattern:** Keep high-level in mod.rs, extract thematic submodules

### game_loop.rs Split (Proposed)
- **Before:** 3,455 lines in single file
- **After:** 8 focused modules (largest: priority.rs ~600 lines)
- **Pattern:** Extract by responsibility (steps, combat, priority, actions, logging, snapshot)

### Key Differences
- **game_loop.rs** has more logical boundaries (steps, combat, priority are distinct phases)
- **actions.rs** kept more in mod.rs (70KB), we're splitting game_loop more aggressively
- **priority.rs** will be the "heavy" module (~600 lines) - still much smaller than actions/mod.rs

## Benefits of Refactoring

1. **Maintainability**
   - Each module <600 lines (manageable size)
   - Clear responsibilities
   - Easier to locate code

2. **Testing**
   - Module-specific test files
   - Isolated testing of priority, combat, steps
   - Easier to write focused tests

3. **Collaboration**
   - Multiple developers can work on different modules
   - Reduced merge conflicts

4. **Comprehension**
   - New developers can understand one module at a time
   - Clear dependency graph

5. **Performance**
   - Parallel compilation of modules
   - Easier to profile specific areas

## Risks and Mitigations

### Risk 1: Breaking Existing Tests
**Mitigation:**
- Run full test suite after each module extraction
- Keep integration tests at top level
- Verify `make validate` passes at each step

### Risk 2: Circular Dependencies
**Mitigation:**
- Careful planning of module dependencies
- Use trait abstraction if needed
- Extract logging/actions first (no dependencies)

### Risk 3: Performance Regression
**Mitigation:**
- Run benchmarks before/after
- Use `#[inline]` where appropriate for small helpers
- Monitor compilation time

### Risk 4: priority.rs Too Complex
**Observation:** `priority_round()` is 544 lines and very complex

**Potential Further Split:**
- `priority/mod.rs` - Main loop
- `priority/spell_abilities.rs` - Ability collection
- `priority/replay.rs` - Replay mode handling
- `priority/resolution.rs` - Stack resolution

**Recommendation:** Start with single priority.rs, evaluate after extraction

## Testing Strategy

### Pre-Refactoring
1. Document current test coverage
2. Run `make validate` and capture baseline
3. Run benchmarks (if available) to establish performance baseline
4. Create snapshot of git state

### During Refactoring
1. After each module extraction:
   - Run `make validate`
   - Verify no compiler warnings
   - Run targeted tests for extracted module
2. Commit after each successful module extraction
3. Write descriptive commit messages explaining what was moved

### Post-Refactoring
1. Full regression test suite
2. Integration tests with real decks
3. Performance benchmarks comparison
4. Update documentation

## Recommended Commit Sequence

```
1. refactor(game_loop): Extract logging module for display and formatting
2. refactor(game_loop): Extract actions module for availability queries
3. refactor(game_loop): Extract snapshot module for save/load functionality
4. refactor(game_loop): Extract legacy v1 PlayerAction interface (mark deprecated)
5. refactor(game_loop): Extract steps module for turn/step execution
6. refactor(game_loop): Extract combat module for combat phase handling
7. refactor(game_loop): Extract priority module for priority system and stack
8. refactor(game_loop): Consolidate mod.rs with core GameLoop and orchestration
9. docs(game_loop): Update README for new module structure
10. test(game_loop): Add module-specific test coverage
```

## Related Work

- Similar to recent actions.rs refactoring (commit e4797e44)
- Follow patterns from actions/ submodule structure
- Maintain consistency with project coding conventions (DRY, strong types, zero-copy)

## Next Steps

1. Review this analysis with maintainers
2. Get approval for refactoring approach
3. Create beads issue to track refactoring work
4. Execute refactoring in phases (low-risk first)
5. Update documentation as modules are extracted

## Conclusion

The game_loop.rs file is ripe for refactoring. At 3,455 lines with 66 methods, it violates the project's guideline of keeping files under 1,500 lines. The proposed 8-module split follows natural architectural boundaries (steps, combat, priority, actions, logging, snapshot, legacy) and will significantly improve maintainability.

**Priority Module** (priority.rs) is the critical path - it contains the complex priority_round() logic and should be extracted with extreme care and thorough testing.

The refactoring should follow the proven pattern from actions.rs: extract modules incrementally, test at each step, and commit frequently.
