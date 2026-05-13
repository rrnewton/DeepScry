---
title: Decouple game.html from ratzilla — incremental migration to ratzilla-free renderer
status: open
priority: 1
issue_type: task
created_at: 2026-05-13T01:00:49.521576282+00:00
updated_at: 2026-05-13T02:09:27.776029120+00:00
---

# Description

Track the 6-step plan to decouple `web/game.html` from the hidden ratzilla
terminal it currently launches. Design doc lives as tg note on
`design-gamehtml-decouple` (also captured in `ai_docs/` if extracted).

## Background

`game.html` already behaves as a thin DOM renderer over a Rust view-model
(`tui_get_gui_view_model_json`) but it used to launch the full ratzilla TUI
behind a hidden `<div id="ratzilla-terminal" style="display:none">`. The
hidden ratzilla loop was doing two things `game.html` actually relied on:

1. **Ticking** — `draw_web` was what called `should_auto_run` →
   `run_until_choice` and fired `window.onRenderComplete`.
2. **Routing Up/Down/Enter** — these keys were deliberately not bound in JS
   because they flowed through `terminal.on_key_event` → `process_key_event`
   → `select_previous_choice/select_next_choice/select_current_choice`.

Everything else (clicks on cards, prompt buttons, Space/A/Q/digits, the
entire DOM render) called ratzilla-free WASM exports.

After step 4, **game.html no longer touches ratzilla at all** — the hidden
div is gone, `launch_game_session()` (decouple-step3) is the only entry
point, and all keyboard / mouse / tick wiring is JS-side.

## 6-Step Migration Plan

- **Step 1 — DONE** (commit 7c18f3a9):
  - Added `tui_tick() -> bool`, `fire_render_complete_callback`,
    `with_state_mut_notify`. Pure-logic mutators notify JS without
    needing the ratzilla draw_web tick.

- **Step 2 — DONE** (commit 199c7dfd):
  - Bound ArrowUp/ArrowDown/Enter in `web/game.html`'s keydown listener;
    `e.stopImmediatePropagation()` prevented ratzilla's also-document
    listener from double-firing.

- **Step 3 — DONE** (commit 193f9c35):
  - Extracted `install_global_session`, `attach_ratzilla_renderer`.
  - New WASM export `launch_game_session(...)` creates the session
    WITHOUT touching ratzilla.
  - `launch_fancy_tui` is now `launch_game_session +
    attach_ratzilla_renderer`.
  - New e2e test `web/test_decouple_step3_launch_game_session.js`
    drives a 13-turn game with `#ratzilla-terminal` removed from the DOM.

- **Step 4 — DONE** (commit TBD):
  - `web/game.html` imports `launch_game_session` (and `tui_tick`)
    instead of `launch_fancy_tui`. Launch flow now passes 7 args
    (no `_canvas_width` / `_canvas_height`) and uses the
    ratzilla-free entry point.
  - Removed `<div id="ratzilla-terminal">` element from
    `web/game.html` entirely (replaced with explanatory comment).
  - Updated keydown handler comment block: `stopImmediatePropagation`
    on Up/Down/Enter is now a no-op (no other listener) but kept as
    defensive code in case some future feature re-attaches a global
    keydown listener.
  - Fixed a Space-handler regression that surfaced when ratzilla
    went away: pre-step-4 the hidden ratzilla terminal converted
    Space at a human choice point into `select_current_choice`
    (commit + advance), while JS-side `tui_run_turn` only calls
    `run_until_choice` (which immediately re-blocks on the same
    prompt). Updated game.html's Space handler to call
    `tui_select_choice() + tui_run_turn()` — `tui_select_choice`
    early-returns when no pending context, so this is safe in
    AI-vs-AI mode too. This restores the legacy UX: Space commits
    the highlighted choice and advances.
  - Bumped `test_game_gui_bugfixes.js` Space-press loop from 3 → 10
    (BUG #4 "log contains draws") to compensate for losing the
    ratzilla Space-double-fire — the test now passes 15/15 again.
  - Updated `test_decouple_step3_launch_game_session.js` to assert
    "ratzilla-terminal element absent from game.html" rather than
    "removed before launch" (it never gets added anymore).
  - Files: `web/game.html`, `web/test_game_gui_bugfixes.js`,
    `web/test_decouple_step3_launch_game_session.js`.
  - `web/fancy.html` is **unchanged** (still uses `launch_fancy_tui`).
  - Verification:
    - `test_game_gui_bugfixes.js`: 15/15 PASS (full bugfix suite,
      including ArrowUp/ArrowDown/Enter and "log contains draws").
    - `test_decouple_step3_launch_game_session.js`: 7/7 PASS,
      including the new "ratzilla-terminal element absent from
      game.html" assertion.
    - `test_fancy_tui.js`: PASS — fancy.html unaffected.
    - `test_click_and_log.js`: PASS — game.html click handling still
      works without ratzilla.
    - `make validate`: GREEN end-to-end.

- **Step 5** — Split `FancyTuiState` into `GameUiSessionState` (shared:
  selected_card_id, valid_choices, choice_context, highlighted_choice,
  digit_buffer, rewind_message) and `RatatuiViewState` (ratatui-only:
  focused_pane, per-pane indices, entity_positions, log scroll, wrap
  cache, pane areas).

- **Step 6** — Fix latent `valid_choices` wire-up bug in WASM. Currently
  populated only by NATIVE controllers; `WasmHumanController` does not
  write it, so `is_valid_choice` highlighting in the GUI silently
  always renders false.

## Files Affected (for reference)

- `mtg-engine/src/wasm/fancy_tui.rs` (split, no logic changes)
- `mtg-engine/src/game/fancy_tui_renderer.rs` (extract RatatuiViewState)
- `mtg-engine/src/game/fancy_tui_events.rs` (split handlers)
- `mtg-engine/src/wasm/gui_view_model.rs` (accept valid_choices from session)
- `web/game.html` (DONE in step 4)
- `web/fancy.html` (no behavioural changes)

## Verification per step

Each step compiles, passes `make validate`, and leaves both fancy.html
and game.html working. WASM E2E (`web/test_*.js`) should stay green.

## Step 4 evidence (2026-05-12, commit TBD)

- `web/test_game_gui_bugfixes.js`: 15/15 PASS — includes the
  re-toleranced "Log contains draw actions" assertion (P1 draws Wheel
  of Fortune from seed-42 game flow appears within 10 Space presses).
- `web/test_decouple_step3_launch_game_session.js`: 7/7 PASS — first
  assertion now reads "ratzilla-terminal element absent from
  game.html (decouple-step4)" and passes because game.html ships
  without the div.
- `web/test_fancy_tui.js`: PASS — fancy.html still uses
  `launch_fancy_tui` and works unchanged.
- `web/test_click_and_log.js`: PASS — clicks still update the card
  details pane via `tui_select_card` →
  `fire_render_complete_callback` → `window.onRenderComplete`.
- `make validate`: GREEN end-to-end (749 lib tests + WASM E2E +
  network E2E + agentplay).
- Source verification: `grep -n "ratzilla\|RatZilla" web/game.html`
  returns ONLY a comment block explaining why the div was removed
  (no live code path uses it).
