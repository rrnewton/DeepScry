---
title: Web GUI undo/rewind architecture bugs
status: open
priority: 2
issue_type: task
created_at: 2026-01-13T15:16:26.717056659+00:00
updated_at: 2026-01-13T15:16:26.717056659+00:00
---

# Description

## Web GUI Undo/Rewind Architecture Bugs

Multiple related bugs in the Web GUI stemming from the undo/rewind architecture corrupting game state.

## Bug 1: Every Other Turn Missing from Log

**Symptom:** Only even-numbered turns show in the log (Turn 8, Turn 10). Turn 11 shows temporarily but gets rewound and deleted from logging memory.

**Root cause:** The `ChangeTurn` action was logged with `prior_log_size` captured BEFORE the turn separator was logged. When rewinding to turn start and truncating the log to `prior_log_size`, the turn separator got removed.

**FIX APPLIED (2026-01-13):** Moved turn separator logging BEFORE capturing `prior_log_size` in `state.rs:advance_step()`. Now when we rewind and truncate, the turn separator is preserved.

- [x] Fixed in `mtg-engine/src/game/state.rs`

## Bug 2: Lightning Strike Invalid Game Action

**Symptom:** Random opponent attempts Lightning Strike in web GUI game and gets "invalid game action" error.

**Investigation result:** Confirmed Lightning Strike works correctly in TUI random vs random games. The bug is specific to web GUI (WASM) mode. May be related to reveal draining timing or stale target information after rewind.

**Status:** Still investigating - may be related to Bug 3

## Bug 3: Heartless Act Double Targeting / No Effect

**Symptom:** In ryan_avatar deck mirror game in web GUI:
1. Cast Heartless Act on opponent's creature
2. Asked the same targeting question twice
3. Creature not actually killed

**Root cause hypothesis:** Same undo/rewind architecture issue corrupting game state and causing duplicate prompts. May need to track which choices have been submitted to prevent re-asking.

**Status:** Still investigating

## Proposed Infrastructure Improvement

Add debug state tracking for web GUI (similar to `--debug-network`):

1. **Action count monotonicity**: Action count should only increase, never decrease
2. **Log append-only invariant**: Game log should only grow, entries should never be removed
3. **Fatal error on violation**: If invariants fail, signal clear fatal error with details

This would allow automated Playwright e2e tests to catch these regressions.

**FIX APPLIED (2026-01-13):** Added monotonicity invariants to `WasmFancyTuiState`:
- `high_water_action_count` and `high_water_log_count` track maximum values
- `in_rewind_replay` flag suppresses checks during replay
- `check_monotonicity_invariants()` verifies monotonicity after each game loop run
- Violations set `error_message` and `game_over` for clear reporting

- [x] Monotonicity invariants added with clear error reporting

## Files Modified

- `mtg-engine/src/game/state.rs` - Fixed turn separator ordering
- `mtg-engine/src/wasm/fancy_tui.rs` - Added monotonicity invariants

## Acceptance Criteria

- [x] All turns appear in log correctly (Bug 1 fixed)
- [ ] Lightning Strike works in web GUI games (still investigating)
- [ ] Heartless Act targets once and resolves correctly (still investigating)
- [x] Monotonicity invariants added with clear error reporting
- [ ] Playwright e2e tests catch invariant violations (manual testing needed)
