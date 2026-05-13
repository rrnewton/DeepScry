---
title: Decouple game.html from ratzilla — incremental migration to ratzilla-free renderer
status: open
priority: 1
issue_type: task
created_at: 2026-05-13T01:00:49.521576282+00:00
updated_at: 2026-05-13T01:48:25.567546150+00:00
---

# Description

Track the 6-step plan to decouple `web/game.html` from the hidden ratzilla
terminal it currently launches. Design doc lives as tg note on
`design-gamehtml-decouple` (also captured in `ai_docs/` if extracted).

## Background

`game.html` already behaves as a thin DOM renderer over a Rust view-model
(`tui_get_gui_view_model_json`) but it still launches the full ratzilla TUI
behind a hidden `<div id="ratzilla-terminal" style="display:none">`. The
hidden ratzilla loop is doing two things `game.html` actually relies on:

1. **Ticking** — `draw_web` is what calls `should_auto_run` →
   `run_until_choice` and fires `window.onRenderComplete`.
2. **Routing Up/Down/Enter** — these keys are deliberately not bound in JS
   because they flow through `terminal.on_key_event` → `process_key_event`
   → `select_previous_choice/select_next_choice/select_current_choice`.

Everything else (clicks on cards, prompt buttons, Space/A/Q/digits, the
entire DOM render) calls ratzilla-free WASM exports.

## 6-Step Migration Plan

- **Step 1 — DONE** (commit 7c18f3a9):
  - Added `tui_tick() -> bool` (drives `should_auto_run` + `run_until_choice`).
  - Added `fire_render_complete_callback` + `with_state_mut_notify`
    helpers; wired all pure-logic mutators so the redraw signal stops
    being conditional on ratzilla.

- **Step 2 — DONE** (commit 199c7dfd):
  - Bound ArrowUp/ArrowDown/Enter in `web/game.html` to call
    `tui_prev_choice` / `tui_next_choice` / `tui_select_choice` directly.
  - Used `e.stopImmediatePropagation()` on those three keys to prevent
    ratzilla's also-document-level keydown listener from double-firing.
  - Other keys deliberately left unchanged to keep step 2 scoped.

- **Step 3 — DONE** (commit TBD):
  - Extracted `install_global_session(state)`, `attach_ratzilla_renderer()`
    helpers from `setup_terminal_and_render` /  `launch_fancy_tui`.
  - New WASM export `launch_game_session(card_db, p1_deck, p2_deck,
    starting_life, seed, p1_ctrl, p2_ctrl) -> Result<(), JsValue>` —
    creates the session WITHOUT touching ratzilla. Exposed in
    `pkg/mtg_forge_rs.d.ts: export function launch_game_session(...)`.
  - `launch_fancy_tui` is now a thin two-step wrapper:
    `launch_game_session(...) + attach_ratzilla_renderer()`.
  - `launch_network_game` uses the same `install_global_session +
    attach_ratzilla_renderer` pattern.
  - New JS smoke test `web/test_decouple_step3_launch_game_session.js`
    DELETES the `<div id="ratzilla-terminal">` from the DOM, then calls
    `launch_game_session` + drives the game forward 30 turns of
    Heuristic-vs-Heuristic via `tui_run_turn` / `tui_tick` and confirms
    the view-model JSON shows the game progressed (turn 1 → 13, GAME
    OVER reached). Wired into `make validate` via
    `validate-wasm-e2e-step` in the Makefile.
  - Files: `mtg-engine/src/wasm/fancy_tui.rs` (refactor + new export),
    `Makefile` (add new test), `web/test_decouple_step3_launch_game_session.js` (new).

- **Step 4** — Switch game.html to `launch_game_session`. Delete the
  hidden `<div id="ratzilla-terminal">`. Drop `tui_set_cell_dimensions`
  calls. Drive UI tick from `requestAnimationFrame(tui_tick)` or the
  existing `setTimeout` autorun chain. At this point the
  `stopImmediatePropagation()` calls from step 2 become harmless
  no-ops. Also a good time to revisit the Space/A/digit keys that step
  2 left untouched.

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
- `web/game.html` (drop hidden ratzilla div, switch launcher, bind Up/Down/Enter)
- `web/fancy.html` (no behavioural changes)

## Verification per step

Each step compiles, passes `make validate`, and leaves both fancy.html
and game.html working. WASM E2E (`web/test_*.js`) should stay green.

## Step 1 evidence (2026-05-12, commit 7c18f3a9)

- 749 lib tests pass; clippy clean (native + wasm targets).
- `make validate` green: WASM E2E "Made 20 choices to advance game ...
  PASS: Click events work" with the new mutator → JS callback wiring.
- `pkg/mtg_forge_rs.d.ts` exports `tui_tick(): boolean`.

## Step 2 evidence (2026-05-12, commit 199c7dfd)

- `web/test_game_gui_bugfixes.js`: 15/15 PASS, identical to baseline.
- `make wasm-test-game-gui-playtest`: both seeds pass, 0 bugs/errors.
- `make validate` green end-to-end.

## Step 3 evidence (2026-05-12, layout-engine)

- `web/test_decouple_step3_launch_game_session.js` (new, wired into
  `validate-wasm-e2e-step`) — 7/7 PASS:
    PASS: ratzilla-terminal element removed before launch
    PASS: ratzilla-terminal element absent after removal
    PASS: launch_game_session callable + drives game without ratzilla
          — initialTurn=1 → finalTurn=13
    PASS: view model JSON contained turn_number — final turn_number = 13
    PASS: view model JSON contained status_text
          — final status_text = Turn 13 | Phase: CombatDamage |
            Active: P1 | GAME OVER
    PASS: tui_tick returns a boolean
          — typeof tui_tick() = boolean, sample values: true, false
    PASS: game state advanced after run_turn calls (turn number grew)
          — turn 1 → 13
    PASS: no browser pageerrors / console errors during the run — clean
- `make validate` green; cargo fmt clean; both clippy invocations clean.
- `pkg/mtg_forge_rs.d.ts` now exports `launch_game_session(...)`
  alongside the existing `launch_fancy_tui(...)`.
