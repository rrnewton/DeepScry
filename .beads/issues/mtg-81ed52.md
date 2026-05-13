---
title: Decouple game.html from ratzilla — incremental migration to ratzilla-free renderer
status: open
priority: 1
issue_type: task
created_at: 2026-05-13T01:00:49.521576282+00:00
updated_at: 2026-05-13T01:27:22.962171706+00:00
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
  - Files: `mtg-engine/src/wasm/fancy_tui.rs`,
    `mtg-engine/src/wasm/human_controller.rs` (bonus clippy fix).

- **Step 2 — DONE** (commit TBD):
  - Bound ArrowUp/ArrowDown/Enter in `web/game.html`'s document-level
    keydown listener calling `tui_prev_choice` / `tui_next_choice` /
    `tui_select_choice` directly.
  - Used `e.stopImmediatePropagation()` on those three keys to prevent
    ratzilla's *also*-document-level keydown listener (registered later
    in `launch_fancy_tui`) from double-firing them.
  - Other keys (Space, A, q/Q/Esc, c/C, ?, 1-9) intentionally left
    unchanged so this step stays scoped — those have their own
    subtleties step 4 will revisit when ratzilla is removed.
  - Files: `web/game.html` (keyboard handler only).
  - Verified by `web/test_game_gui_bugfixes.js` (15/15 PASS):
    - "Down arrow moves selection by exactly 1"
    - "Second Down arrow moves by exactly 1 more"
    - "Up arrow moves selection back by 1"
    - "Enter key does not double-advance"
    - "Enter key advances game (log grows)"
  - And by `make validate` (full run, green).

- **Step 3** — Extract `launch_game_session` (ratzilla-free launcher).
  Refactor `launch_fancy_tui` body so the game-creation/session-install
  half (`fancy_tui.rs:3127–3140`) is a free function callable without a
  `DomBackend`. `launch_fancy_tui` becomes
  `launch_game_session + attach_ratzilla_renderer`.

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
- Bonus fix: added `#[allow(clippy::wildcard_enum_match_arm)]` to the
  test mod in `mtg-engine/src/wasm/human_controller.rs` (mirrors
  e3327009's fix to `replay_verifier.rs`) — was a pre-existing CI break
  on `origin/integration` with `cargo clippy --features wasm,network`.

## Step 2 evidence (2026-05-12, layout-engine)

- `web/test_game_gui_bugfixes.js`: 15/15 PASS (all the explicit
  ArrowUp/ArrowDown/Enter assertions verify "moves by 1" and "doesn't
  double-advance").
- `make wasm-test-game-gui-playtest`: both seeds pass (game1_seed42
  turns 1→18, game2_seed123 turns 1→60, 0 bugs, 0 errors).
- `make validate` green end-to-end.
- During development, observed that being too-aggressive with
  `stopImmediatePropagation` (applying to ALL handled keys) regresses
  the "Log contains draw actions" assertion, because the existing
  game.html relies on ratzilla also processing Space — the legacy Space
  double-fire (JS run_until_choice + ratzilla run_until_choice on the
  same keypress) was advancing the game faster than expected. Step 2
  preserves that behavior; step 4 will clean it up holistically.
