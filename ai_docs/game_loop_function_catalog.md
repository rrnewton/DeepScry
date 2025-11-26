# Game Loop Function Catalog

Quick reference for where each function will be located after refactoring.

## Current Location vs. Proposed Module

| Line Range | Function Name | Lines | Current | Proposed Module |
|------------|---------------|-------|---------|-----------------|
| 143-169 | `new()` | 27 | game_loop.rs | mod.rs |
| 170-175 | `with_max_turns()` | 6 | game_loop.rs | mod.rs |
| 176-184 | `with_snapshot_format()` | 9 | game_loop.rs | mod.rs |
| 185-194 | `with_verbosity()` | 10 | game_loop.rs | mod.rs |
| 195-203 | `with_turn_counter()` | 9 | game_loop.rs | mod.rs |
| 204-212 | `with_choice_counter()` | 9 | game_loop.rs | snapshot.rs |
| 213-227 | `with_stop_when_fixed_exhausted()` | 15 | game_loop.rs | snapshot.rs |
| 228-245 | `with_stop_condition()` | 18 | game_loop.rs | snapshot.rs |
| 246-251 | `with_baseline_choice_count()` | 6 | game_loop.rs | snapshot.rs |
| 252-257 | `with_p1_hand_setup()` | 6 | game_loop.rs | mod.rs |
| 258-274 | `with_p2_hand_setup()` | 17 | game_loop.rs | mod.rs |
| 275-302 | `with_replay_mode()` | 28 | game_loop.rs | snapshot.rs |
| 303-317 | `with_verbose()` [deprecated] | 15 | game_loop.rs | mod.rs |
| 318-333 | `reset()` | 16 | game_loop.rs | mod.rs |
| 334-379 | `log_choice_point()` | 46 | game_loop.rs | snapshot.rs |
| 380-418 | `check_stop_conditions()` | 39 | game_loop.rs | snapshot.rs |
| 419-471 | `run_game()` | 53 | game_loop.rs | mod.rs |
| 472-493 | `run_turns()` | 22 | game_loop.rs | mod.rs |
| 494-524 | `count_filtered_choices()` | 31 | game_loop.rs | snapshot.rs |
| 525-622 | `setup_game()` | 98 | game_loop.rs | mod.rs |
| 623-676 | `assert_valid_stopping_point()` | 54 | game_loop.rs | snapshot.rs |
| 677-771 | `save_snapshot_and_exit()` | 95 | game_loop.rs | snapshot.rs |
| 772-798 | `notify_game_end()` | 27 | game_loop.rs | mod.rs |
| 799-837 | `run_turn_once()` | 39 | game_loop.rs | mod.rs |
| 838-946 | `run_turn()` | 109 | game_loop.rs | mod.rs |
| 947-958 | `get_player_name()` | 12 | game_loop.rs | logging.rs |
| 959-976 | `step_name()` | 18 | game_loop.rs | logging.rs |
| 977-1080 | `print_battlefield_state()` | 104 | game_loop.rs | logging.rs |
| 1081-1097 | `print_step_header_if_needed()` | 17 | game_loop.rs | logging.rs |
| 1098-1104 | `should_print_to_stdout()` | 7 | game_loop.rs | logging.rs |
| 1105-1114 | `log_normal()` | 10 | game_loop.rs | logging.rs |
| 1115-1124 | `log_verbose()` | 10 | game_loop.rs | logging.rs |
| 1125-1131 | `log_minimal()` | 7 | game_loop.rs | logging.rs |
| 1132-1345 | `log_effect_execution()` | 214 | game_loop.rs | logging.rs |
| 1346-1364 | `reset_turn_state()` | 19 | game_loop.rs | mod.rs |
| 1365-1398 | `execute_step()` | 34 | game_loop.rs | mod.rs |
| 1399-1428 | `untap_step()` | 30 | game_loop.rs | steps.rs |
| 1429-1475 | `check_phase_triggers()` | 47 | game_loop.rs | priority.rs |
| 1476-1491 | `upkeep_step()` | 16 | game_loop.rs | steps.rs |
| 1492-1545 | `draw_step()` | 54 | game_loop.rs | steps.rs |
| 1546-1558 | `main_phase()` | 13 | game_loop.rs | steps.rs |
| 1559-1569 | `begin_combat_step()` | 11 | game_loop.rs | combat.rs |
| 1570-1686 | `declare_attackers_step()` | 117 | game_loop.rs | combat.rs |
| 1687-1802 | `declare_blockers_step()` | 116 | game_loop.rs | combat.rs |
| 1803-1838 | `combat_damage_step()` | 36 | game_loop.rs | combat.rs |
| 1839-1864 | `has_first_strike_combat()` | 26 | game_loop.rs | combat.rs |
| 1865-1937 | `log_combat_damage()` | 73 | game_loop.rs | combat.rs or logging.rs |
| 1938-1952 | `end_combat_step()` | 15 | game_loop.rs | combat.rs |
| 1953-1967 | `end_step()` | 15 | game_loop.rs | steps.rs |
| 1968-2095 | `cleanup_step()` | 128 | game_loop.rs | steps.rs |
| 2096-2215 | `resolve_top_spell_from_stack()` | 120 | game_loop.rs | priority.rs |
| 2216-2759 | `priority_round()` ⚠️ | 544 | game_loop.rs | priority.rs |
| 2760-2784 | `get_available_attackers()` [legacy] | 25 | game_loop.rs | legacy.rs |
| 2785-2821 | `get_available_blockers()` [legacy] | 37 | game_loop.rs | legacy.rs |
| 2822-2882 | `get_available_actions()` [legacy] | 61 | game_loop.rs | legacy.rs |
| 2883-2918 | `get_available_attacker_creatures()` | 36 | game_loop.rs | actions.rs |
| 2919-2939 | `get_available_blocker_creatures()` | 21 | game_loop.rs | actions.rs |
| 2940-2944 | `get_current_attackers()` | 5 | game_loop.rs | actions.rs |
| 2945-2961 | `get_lands_in_hand()` | 17 | game_loop.rs | actions.rs |
| 2962-3035 | `get_castable_spells()` | 74 | game_loop.rs | actions.rs |
| 3036-3147 | `get_activatable_abilities()` | 112 | game_loop.rs | actions.rs |
| 3148-3202 | `get_available_spell_abilities()` | 55 | game_loop.rs | actions.rs |
| 3203-3263 | `execute_action()` [legacy] | 61 | game_loop.rs | legacy.rs |
| 3264-3321 | `describe_action()` [legacy] | 58 | game_loop.rs | legacy.rs |
| 3322-3355 | `check_win_condition()` | 34 | game_loop.rs | mod.rs |
| 3356-3365 | `spell_requires_stack_target()` | 10 | game_loop.rs | priority.rs |

## Module Summary

### mod.rs (~800 lines)
**Purpose:** Core GameLoop struct, orchestration, builders

**Functions (19):**
- `new()` - Constructor
- `with_max_turns()` - Builder
- `with_snapshot_format()` - Builder
- `with_verbosity()` - Builder
- `with_turn_counter()` - Builder
- `with_p1_hand_setup()` - Builder
- `with_p2_hand_setup()` - Builder
- `with_verbose()` - Builder [deprecated]
- `reset()` - Reset state
- `run_game()` - Main loop
- `run_turns()` - Bounded execution
- `setup_game()` - Game initialization
- `notify_game_end()` - End game notification
- `run_turn_once()` - Single turn
- `run_turn()` - Turn execution
- `reset_turn_state()` - Per-turn reset
- `execute_step()` - Step dispatcher
- `check_win_condition()` - Win detection

**Plus:** GameLoop struct (17 fields), enums (VerbosityLevel, GameResult, GameEndReason)

---

### steps.rs (~400 lines)
**Purpose:** Non-combat turn steps

**Functions (6):**
- `untap_step()` - Untap permanents
- `upkeep_step()` - Upkeep triggers + priority
- `draw_step()` - Draw card + priority
- `main_phase()` - Main phase priority
- `end_step()` - End step + priority
- `cleanup_step()` - Discard, damage removal, end-of-turn

**Dependencies:** `priority::priority_round()`

---

### combat.rs (~450 lines)
**Purpose:** Combat phase implementation

**Functions (7):**
- `begin_combat_step()` - Begin combat + priority
- `declare_attackers_step()` - Attacker selection (117 lines)
- `declare_blockers_step()` - Blocker selection (116 lines)
- `combat_damage_step()` - Apply damage
- `has_first_strike_combat()` - First strike detection
- `log_combat_damage()` - Combat damage logging (73 lines)
- `end_combat_step()` - End combat + priority

**Dependencies:**
- `priority::priority_round()`
- `snapshot::check_stop_conditions()`
- `actions::get_available_attacker_creatures()`
- `actions::get_available_blocker_creatures()`

---

### priority.rs (~600 lines) ⚠️ CRITICAL
**Purpose:** Priority system, stack resolution, triggers

**Functions (4):**
- `priority_round()` - **544 lines!** Main priority loop
- `resolve_top_spell_from_stack()` - Spell resolution (120 lines)
- `check_phase_triggers()` - Triggered abilities (47 lines)
- `spell_requires_stack_target()` - Counterspell check (10 lines)

**Dependencies:**
- `actions::get_available_spell_abilities()`
- `snapshot::check_stop_conditions()`
- `snapshot::log_choice_point()`
- `logging::log_effect_execution()`

---

### actions.rs (~350 lines)
**Purpose:** Available action queries (read-only)

**Functions (7):**
- `get_available_attacker_creatures()` - Can attack (36 lines)
- `get_available_blocker_creatures()` - Can block (21 lines)
- `get_current_attackers()` - Current attackers (5 lines)
- `get_lands_in_hand()` - Playable lands (17 lines)
- `get_castable_spells()` - Spells with mana (74 lines)
- `get_activatable_abilities()` - Usable abilities (112 lines)
- `get_available_spell_abilities()` - Combined list (55 lines)

**Dependencies:** None (uses GameLoop.mana_engine field)

---

### snapshot.rs (~300 lines)
**Purpose:** Snapshot save/load, replay mode, stop conditions

**Functions (8 + builders):**
- `with_choice_counter()` - Builder
- `with_stop_when_fixed_exhausted()` - Builder
- `with_stop_condition()` - Builder
- `with_baseline_choice_count()` - Builder
- `with_replay_mode()` - Builder
- `log_choice_point()` - Log controller choices (46 lines)
- `check_stop_conditions()` - Pre-choice snapshot check (39 lines)
- `count_filtered_choices()` - Choice filtering (31 lines)
- `assert_valid_stopping_point()` - Validation (54 lines)
- `save_snapshot_and_exit()` - Persist to disk (95 lines)

**Dependencies:** Uses GameLoop.undo_log, GameLoop fields

---

### logging.rs (~450 lines)
**Purpose:** Display and formatting (leaf module)

**Functions (9):**
- `get_player_name()` - Player display name (12 lines)
- `step_name()` - Step display name (18 lines)
- `print_battlefield_state()` - **104 lines** - Detailed state
- `print_step_header_if_needed()` - Lazy header (17 lines)
- `should_print_to_stdout()` - Output gate (7 lines)
- `log_normal()` - Normal logging (10 lines)
- `log_verbose()` - Verbose logging (10 lines)
- `log_minimal()` - Minimal logging (7 lines)
- `log_effect_execution()` - **214 lines** - Effect logging

**Dependencies:** None (delegates to game.logger)

---

### legacy.rs (~250 lines) [DEPRECATED]
**Purpose:** Legacy v1 PlayerAction interface (to be removed)

**Functions (5):**
- `get_available_attackers()` - v1 attackers (25 lines)
- `get_available_blockers()` - v1 blockers (37 lines)
- `get_available_actions()` - v1 action list (61 lines)
- `execute_action()` - v1 execution (61 lines)
- `describe_action()` - v1 descriptions (58 lines)

**Plus:** PlayerAction enum

**Status:** Mark entire module `#[deprecated]`, remove in future commit

---

## Complexity Rankings

### By Line Count (Top 10)
1. `priority_round()` - 544 lines ⚠️
2. `log_effect_execution()` - 214 lines
3. `cleanup_step()` - 128 lines
4. `resolve_top_spell_from_stack()` - 120 lines
5. `declare_attackers_step()` - 117 lines
6. `declare_blockers_step()` - 116 lines
7. `get_activatable_abilities()` - 112 lines
8. `run_turn()` - 109 lines
9. `print_battlefield_state()` - 104 lines
10. `setup_game()` - 98 lines

### By Complexity (Subjective)
1. ⚠️⚠️⚠️ `priority_round()` - Very High (priority system core)
2. ⚠️⚠️ `declare_attackers_step()` - High (controller interaction, replay)
3. ⚠️⚠️ `declare_blockers_step()` - High (controller interaction, replay)
4. ⚠️⚠️ `cleanup_step()` - High (discard logic, SBA)
5. ⚠️ `resolve_top_spell_from_stack()` - Medium-High (effect execution)
6. ⚠️ `run_turn()` - Medium-High (turn orchestration)
7. ⚠️ `get_castable_spells()` - Medium (mana validation)
8. ⚠️ `get_activatable_abilities()` - Medium (ability validation)

### By Risk Level (Extraction)
1. 🔴 `priority_round()` - Highest risk (core engine logic)
2. 🟡 `declare_attackers_step()` - Medium risk (complex interactions)
3. 🟡 `declare_blockers_step()` - Medium risk (complex interactions)
4. 🟡 `cleanup_step()` - Medium risk (SBA interactions)
5. 🟢 All logging functions - Low risk (pure formatting)
6. 🟢 All action queries - Low risk (read-only)
7. 🟢 All snapshot functions - Low risk (self-contained)

## Notes

### Ambiguous Assignments
- `log_combat_damage()` - Could go in combat.rs OR logging.rs
  - **Recommendation:** combat.rs (domain-specific logging)
  - Alternative: logging.rs if we want all logging centralized

### Builder Methods Split
Builder methods split between mod.rs and snapshot.rs:
- **mod.rs:** Game configuration (max_turns, verbosity, hand_setup)
- **snapshot.rs:** Snapshot configuration (stop conditions, replay mode)

This keeps concerns separated but requires re-export from mod.rs:
```rust
// In mod.rs
pub use snapshot::{with_choice_counter, with_stop_condition, ...};
```

### Legacy Module
The legacy.rs module is entirely deprecated. Consider:
1. Extract it now, mark `#[deprecated]`
2. Remove in a future commit once v2 interface is fully tested
3. Or skip extraction and delete directly if no longer used

## Testing Checklist

After extracting each module:

- [ ] `make validate` passes
- [ ] No compiler warnings
- [ ] Module-specific tests added (if applicable)
- [ ] Integration tests pass
- [ ] Benchmark comparison (no regression)
- [ ] Git commit with descriptive message

## References

- Main analysis: `game_loop_refactoring_analysis.md`
- Dependencies: `game_loop_module_dependencies.txt`
- Original file: `mtg-engine/src/game/game_loop.rs` (3,455 lines)
- Similar refactoring: actions.rs (commit e4797e44)
