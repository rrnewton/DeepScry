---
title: Decouple native_game.html from ratzilla — incremental migration to ratzilla-free renderer
status: open
priority: 1
issue_type: task
created_at: 2026-05-13T01:00:49.521576282+00:00
updated_at: 2026-05-13T03:47:13.197437852+00:00
---

# Description

Track the 6-step plan to decouple `web/native_game.html` from the hidden ratzilla
terminal it currently launches. **ALL 6 STEPS COMPLETE.**

## 6-Step Migration Plan

- **Step 1 — DONE** (commit 7c18f3a9 → rebased ...): Added `tui_tick()`,
  `fire_render_complete_callback`, `with_state_mut_notify`. Pure-logic
  mutators notify JS without the ratzilla draw_web tick.

- **Step 2 — DONE** (commit 199c7dfd → rebased ...): Bound
  ArrowUp/ArrowDown/Enter in web/native_game.html with
  `e.stopImmediatePropagation()`.

- **Step 3 — DONE** (commit 193f9c35 → rebased ...): Extracted
  `install_global_session`, `attach_ratzilla_renderer`. New WASM
  export `launch_game_session(...)` creates session WITHOUT ratzilla.

- **Step 4 — DONE** (commit d4e028fd → rebased ...): Switched
  web/native_game.html to `launch_game_session`, removed hidden
  ratzilla-terminal div. Fixed Space-handler regression.

- **Step 5 — DONE** (commit 2c250e72 → rebased ...): Split
  `FancyTuiState` into `GameUiSessionState` (shared, NO ratatui types)
  + `RatatuiViewState` (ratatui-only). Updated ~340 access sites.

- **Step 6 — DONE** (commit TBD): Fix latent valid_choices wire-up
  bug in WASM. The native `FancyTuiController` /
  `FancyFixedController` write `state.session.valid_choices` directly
  from inside their `choose_*` methods, but the WASM
  `WasmHumanController` doesn't (it returns `NeedInput(ChoiceContext::*)`
  instead). So `is_valid_choice` highlighting in the GUI silently
  always rendered false — none of the cards the human could pick lit
  up as such.

  **Fix in mtg-engine/src/wasm/fancy_tui.rs:**
  - `valid_choice_cards(&ChoiceContext)` helper extracts the cards from
    each ChoiceContext variant (mirrors the native pattern: SpellAbility
    → available.iter().map(SpellAbility::card_id), Targets →
    valid_targets.clone(), Blockers → blockers + attackers chained, etc.)
  - `renderer_choice_category(&ChoiceContext)` maps the rich controller
    ChoiceContext to the simple renderer-side category enum (PlayingSpell
    / TargetSelection / DeclareAttackers / DeclareBlockers / None) used
    for the native ratatui dim-non-valid-cards rendering.
  - `clear_pending_choice_highlights()` method clears valid_choices
    and resets choice_context to None — called at all 3 game-end sites
    + the network "waiting for server" idle case.
  - `update_choices_from_context()` now writes both
    `state.session.valid_choices` and `state.session.choice_context`
    every time a new ChoiceContext arrives.

  **Verification (web/test_decouple_step6_valid_choices.js, in
  validate-wasm-e2e-step):**
  - Initial state (turn 1, human prompted with 4 actions): 3 valid
    cards highlighted (Bazaar of Baghdad + 2x City of Brass).
  - After 4x Space (advance to turn 3): 8 valid cards highlighted
    (recomputed correctly after each prompt cycle).
  - Pre-step-6 baseline (verified by stashing my changes and
    rebuilding wasm): 0 valid cards in both states. The test fails
    catastrophically without the fix — proves the regression guard
    is real.

## Files Changed (final summary across all 6 steps)

- `mtg-engine/src/wasm/fancy_tui.rs` — main WASM driver, all 6 steps
- `mtg-engine/src/game/fancy_tui_renderer.rs` — step 5 struct split
- `mtg-engine/src/game/fancy_tui_events.rs` — step 5 access updates
- `mtg-engine/src/game/fancy_tui_controller.rs` — step 5 access updates
- `mtg-engine/src/game/fancy_fixed_controller.rs` — step 5 access updates
- `mtg-engine/src/wasm/human_controller.rs` — step 1 clippy fix
- `web/native_game.html` — steps 2 + 4 keyboard + launch + (step 5 layout fix)
- `Makefile` — wired all new e2e tests into validate-wasm-e2e-step
- `web/test_decouple_step3_launch_game_session.js` — new e2e
- `web/test_card_size_stability.js` — new e2e
- `web/test_decouple_step6_valid_choices.js` — new e2e
- `web/test_game_gui_bugfixes.js` — adjusted Space-press loop count
- Bonus pre-existing CI fixes (steps 1, 5):
  `human_controller.rs::tests` allow,
  `main.rs` ungated `use super::*;`,
  `actions.rs` unnested or-patterns,
  `effect_converter.rs` field-reassign rewrite,
  `state.rs` # Errors doc.

The decoupling is complete — native_game.html is fully ratzilla-free, runs
on a clean shared `GameUiSessionState`, and shows the same valid-choice
highlights the native TUI shows.
