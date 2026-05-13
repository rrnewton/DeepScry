---
title: Decouple game.html from ratzilla — incremental migration to ratzilla-free renderer
status: open
priority: 1
issue_type: task
created_at: 2026-05-13T01:00:49.521576282+00:00
updated_at: 2026-05-13T03:22:09.775298631+00:00
---

# Description

Track the 6-step plan to decouple `web/game.html` from the hidden ratzilla
terminal it currently launches. Design doc lives as tg note on
`design-gamehtml-decouple` (also captured in `ai_docs/` if extracted).

## 6-Step Migration Plan

- **Step 1 — DONE** (commit 7c18f3a9 → rebased 34a6d8e0): Added
  `tui_tick()`, `fire_render_complete_callback`,
  `with_state_mut_notify`. Pure-logic mutators notify JS without
  the ratzilla draw_web tick.

- **Step 2 — DONE** (commit 199c7dfd → rebased 42613d9f): Bound
  ArrowUp/ArrowDown/Enter in `web/game.html` to call ratzilla-free
  WASM exports directly with `e.stopImmediatePropagation()`.

- **Step 3 — DONE** (commit 193f9c35 → rebased 17d0a631): Extracted
  `install_global_session`, `attach_ratzilla_renderer` helpers.
  New WASM export `launch_game_session(...)` creates session WITHOUT
  ratzilla. `launch_fancy_tui` is now `launch_game_session +
  attach_ratzilla_renderer`.

- **Step 4 — DONE** (commit d4e028fd → rebased dfdc0e9e): Switched
  `web/game.html` to `launch_game_session`, removed the hidden
  `<div id="ratzilla-terminal">`. Fixed Space-handler regression
  (now calls `tui_select_choice() + tui_run_turn()`).

- **Step 5 — DONE** (commit TBD): Split `FancyTuiState` into
  `GameUiSessionState` (shared: highlighted_choice,
  selected_card_id, valid_choices, choice_context, digit_buffer,
  rewind_message — NO ratatui types) + `RatatuiViewState` (ratatui
  only: focused_pane, selected_card_in_*, entity_positions,
  *_pane_area, log_*, log_wrap_cache, logger_memory_mode_enabled).
  All log_* methods moved from `impl FancyTuiState` to
  `impl RatatuiViewState`. `FancyTuiState` is now a thin wrapper
  `{ pub session: GameUiSessionState, pub view: RatatuiViewState }`.
  Updated ~340 field-access sites across 5 files
  (fancy_tui_renderer.rs, fancy_tui_events.rs, fancy_tui.rs,
  fancy_fixed_controller.rs, fancy_tui_controller.rs).
  Bonus: pre-existing CI fixes in main.rs (test mod `use super::*;`),
  game_loop/actions.rs (unnested or-patterns), effect_converter.rs
  (field_reassign_with_default), state.rs (missing # Errors doc).

- **Step 6** — Fix latent `valid_choices` wire-up bug in WASM
  (`WasmHumanController` doesn't populate it).

## Step 5 evidence (commit TBD)

- 760 lib tests pass; cargo fmt clean; clippy clean on native +
  wasm-tui + wasm,network feature sets.
- web/test_decouple_step3_launch_game_session.js: 7/7 PASS.
- web/test_card_size_stability.js: 10/10 PASS.
- WASM bindings unchanged (visible export surface still
  `launch_game_session`, `tui_tick`, `tui_*_choice`, etc.).
- Pre-existing failures NOT introduced by this commit (verified
  by checking out origin/integration directly):
  - `vinebender_waterbend_test::vinebender_activation_places_p1p1_counter_on_self` — `EntityNotFound(4294967292)`. Likely caused by the chaos-orb / All Hallow's Eve target-resolution refactors on integration.
  - `shell_scripts__commander_e2e` — long-running shell test
    fails on integration too. Out of scope for the WASM/UI
    decoupling work.
