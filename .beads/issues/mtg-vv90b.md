---
title: 'feat: PASS_UNTIL semantic puzzle controller command'
status: open
priority: 3
issue_type: task
created_at: 2026-06-13T23:04:03.476446397+00:00
updated_at: 2026-06-13T23:04:03.476446397+00:00
---

# Description

## Problem

Puzzle scripts using the fixed/rich-input controller are brittle because they reference actions by INDEX (e.g. 'take the 3rd offered action'). Any change to action ordering, new triggers firing, or board state changes breaks the test.

## Solution

Implemented PASS_UNTIL semantic command in the RichInputController and command_parsing module:

### New commands
- `PASS_UNTIL turn=N,phase=PHASE` — pass priority until the game reaches turn N at the named phase
- `PASS_UNTIL phase=PHASE` — pass until the next occurrence of PHASE (any turn)
- `PASS_UNTIL turn=N` — pass until the start of turn N (any phase)

### Implementation details
- Strong-typed `PassUntilCondition` struct with `is_satisfied(turn, step)` method
- `parse_pass_until()` function: proper tokenized parsing (splits on `,` and whitespace), extracts `key=value` pairs, validates against `Step::from_script_name`
- Integrated into `RichInputController::handle_pass_until()` helper called at the head of `choose_spell_ability_to_play`, `choose_attackers`, `choose_blockers`
- Added 'combat' alias to `Step::from_script_name` (maps to BeginCombat — the natural user intent)
- NO parallel decision path — condition-satisfied path just consumes the directive and falls through to normal script execution

### Backward compatibility
All existing index-based scripts continue to work unchanged. PASS_UNTIL is purely additive.

### Information independence
Uses only `view.turn_number()` and `view.current_step()` — public game state. Network-deterministic.

### Tests
- 13 unit tests for PassUntilCondition + parse_pass_until in command_parsing.rs
- 3 integration tests in rich_input_controller.rs (not yet satisfied passes, satisfied consumes, malformed is safe)
- Demo puzzle: test_puzzles/pass_until_semantic_demo.pzl

## Files changed
- mtg-engine/src/game/command_parsing.rs: PassUntilCondition, parse_pass_until, tests
- mtg-engine/src/game/phase.rs: 'combat' alias for BeginCombat
- mtg-engine/src/game/rich_input_controller.rs: handle_pass_until, wired into choose_* methods
- docs/FIXED_INPUT_SYNTAX.md: PASS_UNTIL section with examples and comparison table
- test_puzzles/pass_until_semantic_demo.pzl: demo puzzle using PASS_UNTIL + named cast
